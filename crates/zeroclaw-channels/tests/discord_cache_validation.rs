use std::fs;
use std::path::PathBuf;

use anyhow::{Context, bail};
use regex::Regex;
use serde::Serialize;
use zeroclaw_channels::orchestrator::discord_cache_validation_support::{
    DaemonEquivalentCacheProbe, run_daemon_equivalent_cache_probe,
};

const PRIMARY_EVIDENCE_FILE: &str = "task-8-discord-or-daemon-equivalent.json";
const FALLBACK_EVIDENCE_FILE: &str = "task-8-daemon-equivalent-fallback.json";

#[tokio::test]
#[ignore = "requires explicit Task 8 Discord/cache validation invocation"]
async fn discord_or_daemon_equivalent_cache_validation() -> anyhow::Result<()> {
    let evidence_dir = evidence_dir()?;
    fs::create_dir_all(&evidence_dir).with_context(|| {
        format!(
            "creating evidence directory {}",
            evidence_dir.to_string_lossy()
        )
    })?;

    let discord_token = read_required_env("DISCORD_TOKEN");
    let discord_channel_id = read_required_env("ZEROCLAW_DISCORD_TEST_CHANNEL_ID");
    let deepseek_api_key = read_required_env("ZEROCLAW_DEEPSEEK_API_KEY");

    if discord_token.is_some() && discord_channel_id.is_some() && deepseek_api_key.is_some() {
        let evidence = PrimaryEvidence::live_stub();
        write_json_evidence(&evidence_dir.join(PRIMARY_EVIDENCE_FILE), &evidence)?;
        bail!(
            "Task 8 live Discord/DeepSeek validation support is not implemented in this harness yet; unset at least one of DISCORD_TOKEN, ZEROCLAW_DISCORD_TEST_CHANNEL_ID, or ZEROCLAW_DEEPSEEK_API_KEY to run the daemon-equivalent fallback"
        );
    }

    let skipped = skipped_reason(
        discord_token.is_some(),
        discord_channel_id.is_some(),
        deepseek_api_key.is_some(),
    );
    let probe = run_daemon_equivalent_cache_probe()
        .await
        .context("running daemon-equivalent cache validation through orchestrator")?;

    assert_probe_contract(&probe)?;

    let fallback = FallbackEvidence::from_probe(&probe);
    let primary = PrimaryEvidence::fallback(&skipped, &fallback);
    write_json_evidence(&evidence_dir.join(FALLBACK_EVIDENCE_FILE), &fallback)?;
    write_json_evidence(&evidence_dir.join(PRIMARY_EVIDENCE_FILE), &primary)?;

    println!(
        "DISCORD_TRANSPORT_SKIPPED=true; REASON={}; FALLBACK_PROVIDER_CALLS={}; FIRST_DIFF_MARKER={}; STABLE_PREFIX_TOKENS_ESTIMATE={}; evidence=.sisyphus/evidence/{PRIMARY_EVIDENCE_FILE}; fallback_evidence=.sisyphus/evidence/{FALLBACK_EVIDENCE_FILE}",
        skipped,
        probe.provider_calls.len(),
        probe.first_diff_marker,
        probe.stable_prefix_tokens_estimate
    );

    Ok(())
}

fn assert_probe_contract(probe: &DaemonEquivalentCacheProbe) -> anyhow::Result<()> {
    if probe.provider_calls.len() < 2 {
        bail!(
            "expected at least two provider payload captures, got {}",
            probe.provider_calls.len()
        );
    }
    if probe.synthetic_memory_entries.is_empty() {
        bail!("daemon-equivalent fallback must use non-empty synthetic memory");
    }
    if probe.message_ids[0] == probe.message_ids[1] {
        bail!("daemon-equivalent fallback must vary synthetic message ids");
    }
    if !probe.runtime_command_contract.discord_bang_new_supported
        || !probe
            .runtime_command_contract
            .discord_role_mention_bang_new_supported
    {
        bail!("Discord ! command/runtime mention contract must remain supported");
    }
    if probe.runtime_command_contract.telegram_bang_new_supported {
        bail!("Telegram must not accept Discord-only ! runtime commands");
    }
    if probe.first_diff_marker != "VOLATILE_CONTEXT_OR_CURRENT_USER" {
        bail!(
            "changed volatile inputs must first differ at volatile/current-user boundary, got {}",
            probe.first_diff_marker
        );
    }
    if probe.memory_diff_first_marker != "VOLATILE_CONTEXT_OR_CURRENT_USER" {
        bail!(
            "changed memory must not enter stable prefix, got {}",
            probe.memory_diff_first_marker
        );
    }
    if probe.message_id_diff_first_marker != "VOLATILE_CONTEXT_OR_CURRENT_USER" {
        bail!(
            "changed message id must not enter stable prefix, got {}",
            probe.message_id_diff_first_marker
        );
    }
    if probe.stable_prefix_tokens_estimate < 2_048 {
        bail!(
            "stable prefix estimate must exceed the old prompt-cache floor, got {}",
            probe.stable_prefix_tokens_estimate
        );
    }
    Ok(())
}

fn read_required_env(name: &str) -> Option<String> {
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

fn write_json_evidence<T: Serialize>(path: &std::path::Path, evidence: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(evidence)?;
    fs::write(path, redact(&json)).with_context(|| format!("writing {}", path.display()))
}

fn skipped_reason(
    has_discord_token: bool,
    has_discord_channel_id: bool,
    has_deepseek_api_key: bool,
) -> String {
    let mut missing = Vec::new();
    if !has_discord_token {
        missing.push("DISCORD_TOKEN");
    }
    if !has_discord_channel_id {
        missing.push("ZEROCLAW_DISCORD_TEST_CHANNEL_ID");
    }
    if !has_deepseek_api_key {
        missing.push("ZEROCLAW_DEEPSEEK_API_KEY");
    }
    format!("missing {}", missing.join(","))
}

#[derive(Debug, Serialize)]
struct PrimaryEvidence<'a> {
    result: &'a str,
    discord_transport_status: &'a str,
    discord_transport_reason: &'a str,
    network_attempted: bool,
    fallback_evidence_file: Option<&'a str>,
    fallback: Option<&'a FallbackEvidence>,
}

impl<'a> PrimaryEvidence<'a> {
    fn live_stub() -> Self {
        Self {
            result: "failed",
            discord_transport_status: "LIVE_SUPPORT_NOT_IMPLEMENTED",
            discord_transport_reason: "all live Discord/DeepSeek env vars were present, but this harness only implements the daemon-equivalent fallback path",
            network_attempted: false,
            fallback_evidence_file: None,
            fallback: None,
        }
    }

    fn fallback(discord_transport_reason: &'a str, fallback: &'a FallbackEvidence) -> Self {
        Self {
            result: "passed",
            discord_transport_status: "DISCORD_TRANSPORT_SKIPPED",
            discord_transport_reason,
            network_attempted: false,
            fallback_evidence_file: Some(FALLBACK_EVIDENCE_FILE),
            fallback: Some(fallback),
        }
    }
}

#[derive(Debug, Serialize)]
struct FallbackEvidence {
    result: &'static str,
    validation_path: &'static str,
    provider_calls: usize,
    message_ids_changed: bool,
    synthetic_memory_entries: usize,
    stable_prefix_bytes: usize,
    stable_prefix_tokens_estimate: usize,
    first_diff_marker: String,
    memory_diff_first_marker: String,
    message_id_diff_first_marker: String,
    runtime_command_contract:
        zeroclaw_channels::orchestrator::discord_cache_validation_support::RuntimeCommandContract,
}

impl FallbackEvidence {
    fn from_probe(probe: &DaemonEquivalentCacheProbe) -> Self {
        Self {
            result: "passed",
            validation_path: "orchestrator::process_channel_message -> capture model provider",
            provider_calls: probe.provider_calls.len(),
            message_ids_changed: probe.message_ids[0] != probe.message_ids[1],
            synthetic_memory_entries: probe.synthetic_memory_entries.len(),
            stable_prefix_bytes: probe.stable_prefix_bytes,
            stable_prefix_tokens_estimate: probe.stable_prefix_tokens_estimate,
            first_diff_marker: probe.first_diff_marker.clone(),
            memory_diff_first_marker: probe.memory_diff_first_marker.clone(),
            message_id_diff_first_marker: probe.message_id_diff_first_marker.clone(),
            runtime_command_contract: probe.runtime_command_contract.clone(),
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
        r#"\b\d{15,20}\b"#,
    ];
    for pattern in patterns {
        if let Ok(regex) = Regex::new(pattern) {
            output = regex.replace_all(&output, "<REDACTED>").into_owned();
        }
    }
    output
}
