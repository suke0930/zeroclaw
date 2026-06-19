use std::fs;
use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use regex::Regex;
use serde::Serialize;
use zeroclaw_providers::ChatMessage;
use zeroclaw_providers::compatible::{AuthStyle, OpenAiCompatibleModelProvider};

const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_DEEPSEEK_MODEL: &str = "deepseek-chat";
const OBSERVED_CACHE_FLOOR_TOKENS: u64 = 2_048;
const TASK6_STABLE_PREFIX_TOKENS_ESTIMATE: u64 = 9_148;
const STABLE_PREFIX_CONSISTENT_HIT_THRESHOLD: u64 = 6_400;

#[tokio::test]
#[ignore = "requires ZEROCLAW_DEEPSEEK_API_KEY and performs live DeepSeek calls"]
async fn live_deepseek_prompt_cache_path() -> anyhow::Result<()> {
    let evidence_dir = evidence_dir()?;
    fs::create_dir_all(&evidence_dir).with_context(|| {
        format!(
            "creating evidence directory {}",
            evidence_dir.to_string_lossy()
        )
    })?;

    let Some(api_key) = read_required_env("ZEROCLAW_DEEPSEEK_API_KEY") else {
        write_missing_env_evidence(&evidence_dir)?;
        println!(
            "MISSING_CONFIGURATION=ZEROCLAW_DEEPSEEK_API_KEY; NETWORK_ATTEMPTED=false; evidence=.sisyphus/evidence/task-7-live-missing-env.txt"
        );
        return Ok(());
    };

    let base_url = read_optional_env("ZEROCLAW_DEEPSEEK_BASE_URL")
        .unwrap_or_else(|| DEFAULT_DEEPSEEK_BASE_URL.to_string());
    let model = read_optional_env("ZEROCLAW_DEEPSEEK_MODEL")
        .unwrap_or_else(|| DEFAULT_DEEPSEEK_MODEL.to_string());
    let provider = OpenAiCompatibleModelProvider::new(
        "deepseek-live-cache",
        "DeepSeek",
        &base_url,
        Some(&api_key),
        AuthStyle::Bearer,
    )
    .with_max_tokens(Some(16));

    let first_messages = build_messages("LIVE_SEQUENCE_A");
    let second_messages = build_messages("LIVE_SEQUENCE_B");
    let stable_history_tokens_estimate = stable_history_content().len() as u64 / 4;

    let first = provider
        .chat_with_history_prompt_cache_usage(&first_messages, &model, Some(0.0))
        .await
        .context("sending first DeepSeek prompt-cache probe through compatible provider")?;
    let second = provider
        .chat_with_history_prompt_cache_usage(&second_messages, &model, Some(0.0))
        .await
        .context("sending second DeepSeek prompt-cache probe through compatible provider")?;

    let second_hit = second
        .usage
        .prompt_cache_hit_tokens
        .ok_or_else(|| anyhow!("second DeepSeek response omitted usage.prompt_cache_hit_tokens"))?;
    let second_miss = second.usage.prompt_cache_miss_tokens.ok_or_else(|| {
        anyhow!("second DeepSeek response omitted usage.prompt_cache_miss_tokens")
    })?;

    let current_user_tokens_estimate = current_user_content("LIVE_SEQUENCE_B").len() as u64 / 4;
    let non_current_user_input_tokens_estimate = second
        .usage
        .prompt_tokens
        .and_then(|tokens| tokens.checked_sub(current_user_tokens_estimate))
        .unwrap_or(stable_history_tokens_estimate);
    let seventy_percent_non_current_user = non_current_user_input_tokens_estimate * 70 / 100;
    let stable_prefix_threshold =
        STABLE_PREFIX_CONSISTENT_HIT_THRESHOLD.min(TASK6_STABLE_PREFIX_TOKENS_ESTIMATE * 70 / 100);
    let meets_old_floor = second_hit > OBSERVED_CACHE_FLOOR_TOKENS;
    let meets_percent_threshold = second_hit >= seventy_percent_non_current_user;
    let meets_stable_prefix_threshold = second_hit >= stable_prefix_threshold;

    let evidence = LiveEvidence {
        result: if meets_old_floor && (meets_percent_threshold || meets_stable_prefix_threshold) {
            "passed"
        } else {
            "failed"
        },
        model: &model,
        endpoint: &redact(&base_url),
        provider_path: "OpenAiCompatibleModelProvider::chat_with_history_prompt_cache_usage",
        request_body_bytes: [first.request_body_bytes, second.request_body_bytes],
        stable_history_tokens_estimate,
        task6_stable_prefix_tokens_estimate: TASK6_STABLE_PREFIX_TOKENS_ESTIMATE,
        observed_cache_floor_tokens: OBSERVED_CACHE_FLOOR_TOKENS,
        stable_prefix_consistent_hit_threshold: stable_prefix_threshold,
        non_current_user_input_tokens_estimate,
        seventy_percent_non_current_user,
        second_hit,
        second_miss,
        meets_old_floor,
        meets_percent_threshold,
        meets_stable_prefix_threshold,
        requests: [
            RequestEvidence::from_probe(1, &first),
            RequestEvidence::from_probe(2, &second),
        ],
    };
    write_live_evidence(&evidence_dir, &evidence)?;

    println!(
        "SECOND_PROMPT_CACHE_HIT_TOKENS={second_hit}; SECOND_PROMPT_CACHE_MISS_TOKENS={second_miss}; REQUIRED_OLD_FLOOR>{OBSERVED_CACHE_FLOOR_TOKENS}; SEVENTY_PERCENT_NON_CURRENT_USER_THRESHOLD={seventy_percent_non_current_user}; STABLE_PREFIX_CONSISTENT_HIT_THRESHOLD={stable_prefix_threshold}; PROVIDER_PATH=OpenAiCompatibleModelProvider::chat_with_history_prompt_cache_usage; evidence=.sisyphus/evidence/task-7-deepseek-live-cache.json"
    );

    if !meets_old_floor {
        bail!(
            "second DeepSeek prompt_cache_hit_tokens={second_hit} did not exceed {OBSERVED_CACHE_FLOOR_TOKENS}"
        );
    }
    if !(meets_percent_threshold || meets_stable_prefix_threshold) {
        bail!(
            "second DeepSeek prompt_cache_hit_tokens={second_hit} did not meet 70% non-current-user threshold {seventy_percent_non_current_user} or stable-prefix-consistent threshold {stable_prefix_threshold}"
        );
    }

    Ok(())
}

fn read_required_env(name: &str) -> Option<String> {
    read_optional_env(name).filter(|value| !value.trim().is_empty())
}

fn read_optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn evidence_dir() -> anyhow::Result<PathBuf> {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..3 {
        dir.pop();
    }
    Ok(dir.join(".sisyphus").join("evidence"))
}

fn write_missing_env_evidence(evidence_dir: &std::path::Path) -> anyhow::Result<()> {
    let body = "RESULT=missing configuration\nMISSING=ZEROCLAW_DEEPSEEK_API_KEY\nNETWORK_ATTEMPTED=false\nDETAIL=Set ZEROCLAW_DEEPSEEK_API_KEY and run the ignored test explicitly for live DeepSeek prompt-cache validation.\n";
    fs::write(evidence_dir.join("task-7-live-missing-env.txt"), body)
        .context("writing missing-env DeepSeek live evidence")
}

fn write_live_evidence(
    evidence_dir: &std::path::Path,
    evidence: &LiveEvidence<'_>,
) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(evidence)?;
    fs::write(
        evidence_dir.join("task-7-deepseek-live-cache.json"),
        redact(&json),
    )
    .context("writing DeepSeek live prompt-cache evidence")
}

fn build_messages(sequence_marker: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(stable_system_content()),
        ChatMessage::user(stable_history_content()),
        ChatMessage::assistant(
            "Acknowledged the synthetic stable cache fixture. I will answer with the requested marker only.",
        ),
        ChatMessage::user(current_user_content(sequence_marker)),
    ]
}

fn stable_system_content() -> String {
    "You are validating a synthetic prompt-cache fixture for ZeroClaw. Respond with exactly CACHE_PROBE_ACK and do not mention implementation details.".to_string()
}

fn stable_history_content() -> String {
    let mut content = String::with_capacity(54_000);
    content.push_str("STABLE_HISTORY_MARKER_BEGIN\n");
    for index in 0..560 {
        content.push_str(&format!(
            "Stable cache ledger line {index:04}: quartz raven delta keeps deterministic provider-prefix bytes unchanged across comparable DeepSeek live requests.\n"
        ));
    }
    content.push_str("STABLE_HISTORY_MARKER_END\n");
    content
}

fn current_user_content(sequence_marker: &str) -> String {
    format!(
        "Current user asks for the bounded answer CACHE_PROBE_ACK.\n\n<zeroclaw_current_context>\nSYNTHETIC_VOLATILE_CONTEXT_MARKER={sequence_marker}\nSYNTHETIC_MESSAGE_ID=task7-{sequence_marker}\n</zeroclaw_current_context>"
    )
}

#[derive(Debug, Serialize)]
struct LiveEvidence<'a> {
    result: &'a str,
    model: &'a str,
    endpoint: &'a str,
    provider_path: &'a str,
    request_body_bytes: [usize; 2],
    stable_history_tokens_estimate: u64,
    task6_stable_prefix_tokens_estimate: u64,
    observed_cache_floor_tokens: u64,
    stable_prefix_consistent_hit_threshold: u64,
    non_current_user_input_tokens_estimate: u64,
    seventy_percent_non_current_user: u64,
    second_hit: u64,
    second_miss: u64,
    meets_old_floor: bool,
    meets_percent_threshold: bool,
    meets_stable_prefix_threshold: bool,
    requests: [RequestEvidence; 2],
}

#[derive(Debug, Serialize)]
struct RequestEvidence {
    sequence: u8,
    request_body_bytes: usize,
    text_present: bool,
    usage: zeroclaw_providers::compatible::CompatiblePromptCacheUsage,
}

impl RequestEvidence {
    fn from_probe(
        sequence: u8,
        probe: &zeroclaw_providers::compatible::CompatiblePromptCacheProbe,
    ) -> Self {
        Self {
            sequence,
            request_body_bytes: probe.request_body_bytes,
            text_present: probe.text.as_ref().is_some_and(|text| !text.is_empty()),
            usage: probe.usage,
        }
    }
}

fn redact(input: &str) -> String {
    let mut output = input.to_string();
    let patterns = [
        r#"(?i)bearer\s+[A-Za-z0-9._~+/=-]{8,}"#,
        r#"(?i)(api[_-]?key|authorization|token)[\"'=:\s]+[A-Za-z0-9._~+/=-]{8,}"#,
        r#"(?i)\b(?:sk|dsk|ds)-[A-Za-z0-9_-]{16,}\b"#,
        r#"\bmfa\.[A-Za-z0-9_-]{20,}\b"#,
        r#"\b[A-Za-z0-9_-]{24}\.[A-Za-z0-9_-]{6}\.[A-Za-z0-9_-]{27,}\b"#,
    ];
    for pattern in patterns {
        if let Ok(regex) = Regex::new(pattern) {
            output = regex.replace_all(&output, "<REDACTED>").into_owned();
        }
    }
    output
}
