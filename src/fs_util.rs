use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{METRICS_JSON, RevisionHandoff, SupervisorFeedbackTurn};

pub(crate) fn append_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to append {}", path.display()))?;
    Ok(())
}

pub(crate) fn read_json_file(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

pub(crate) fn read_opencode_session_id_from_metrics(run_dir: &Path) -> Result<Option<String>> {
    let metrics_path = run_dir.join(METRICS_JSON);
    let metrics = read_json_file(&metrics_path)?;
    Ok(get_str(&metrics, "opencode_session_id")
        .filter(|value| value.starts_with("ses_"))
        .map(ToOwned::to_owned))
}

pub(crate) fn supervisor_control_decision_from_metrics(
    run_dir: &Path,
) -> Result<Option<SupervisorFeedbackTurn>> {
    let metrics_path = run_dir.join(METRICS_JSON);
    if !metrics_path.exists() {
        return Ok(None);
    }
    let metrics = read_json_file(&metrics_path)?;
    let Some(events) = metrics
        .get("supervisor_control_events")
        .and_then(Value::as_array)
    else {
        return Ok(None);
    };
    let Some(event) = events.iter().rev().find(|event| {
        matches!(
            get_str(event, "action"),
            Some("interrupt_continue") | Some("interrupt_context_focus") | Some("stop")
        )
    }) else {
        return Ok(None);
    };
    let action = get_str(event, "action").unwrap_or("wait");
    let run_interrupted = get_bool(&metrics, "interrupted_by_supervisor").unwrap_or(false);
    if !run_interrupted
        && matches!(action, "interrupt_continue" | "interrupt_context_focus")
        && (get_u64(&metrics, "changed_file_count").unwrap_or(0) > 0
            || get_u64(&metrics, "patch_bytes").unwrap_or(0) > 0)
    {
        return Ok(None);
    }
    let verdict = if action == "stop" { "stop" } else { "revise" };
    let control = event.get("control").unwrap_or(event);
    let auto_revision_no_delta = get_str(control, "source")
        .is_some_and(|source| source.starts_with("auto_revision_no_delta"));
    let patch_decision = if verdict == "revise" && auto_revision_no_delta {
        "revise_current"
    } else {
        "accept_current"
    };
    let worker_mode = get_str(event, "worker_mode")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if action == "interrupt_context_focus" {
                "context_focus".to_string()
            } else {
                "continue".to_string()
            }
        });
    let hint = get_str(event, "message_to_worker")
        .unwrap_or("")
        .trim()
        .to_string();
    let revision_handoff =
        revision_handoff_from_supervisor_control(control, auto_revision_no_delta, &hint);
    let feedback = json!({
        "label": "supervisor-control",
        "timestamp": Utc::now().to_rfc3339(),
        "source_run": run_dir.to_string_lossy(),
        "verdict": verdict,
        "action": verdict,
        "worker_mode": worker_mode.clone(),
        "patch_decision": patch_decision,
        "message_to_worker": hint,
        "focus_files": get_string_array(event, "focus_files"),
        "required_checks": get_string_array(event, "required_checks"),
        "risk": get_str(event, "risk").unwrap_or(""),
        "supervisor_control_action": action,
        "feedback": event,
    });
    Ok(Some(SupervisorFeedbackTurn {
        feedback,
        verdict: verdict.to_string(),
        worker_mode,
        patch_decision: patch_decision.to_string(),
        hint,
        revision_handoff,
        focus_files: get_string_array(event, "focus_files"),
        required_checks: get_string_array(event, "required_checks"),
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
        thread_id: String::new(),
        turn_id: String::new(),
    }))
}

fn revision_handoff_from_supervisor_control(
    control: &Value,
    auto_revision_no_delta: bool,
    hint: &str,
) -> RevisionHandoff {
    let worker_turn_shape = get_str(control, "worker_turn_shape")
        .map(ToOwned::to_owned)
        .or_else(|| auto_revision_no_delta.then(|| "small_patch_slice".to_string()));
    let mut exact_edits = get_string_array(control, "exact_edits");
    if exact_edits.is_empty() && auto_revision_no_delta && !hint.trim().is_empty() {
        exact_edits.push(hint.trim().to_string());
    }
    RevisionHandoff {
        worker_turn_shape,
        turn_goal: get_str(control, "turn_goal")
            .map(ToOwned::to_owned)
            .or_else(|| {
                auto_revision_no_delta.then(|| "make the first no-delta recovery edit".to_string())
            }),
        exact_edits,
        deferred_checks: get_string_array(control, "deferred_checks"),
        defer_checks_until_patch_exists: get_bool(control, "defer_checks_until_patch_exists")
            .or_else(|| auto_revision_no_delta.then_some(true)),
        completion_gate: get_str(control, "completion_gate")
            .map(ToOwned::to_owned)
            .or_else(|| {
                auto_revision_no_delta.then(|| "git diff --stat must be non-empty".to_string())
            }),
        forbidden_actions: {
            let mut actions = get_string_array(control, "forbidden_actions");
            if auto_revision_no_delta {
                for action in ["ask questions", "run tests before editing"] {
                    if !actions.iter().any(|item| item == action) {
                        actions.push(action.to_string());
                    }
                }
            }
            actions
        },
    }
}

pub(crate) fn get_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

pub(crate) fn env_u64(key: &str) -> Option<u64> {
    env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

pub(crate) fn get_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

pub(crate) fn get_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

pub(crate) fn get_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn first_non_empty_string_array(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .map(|key| get_string_array(value, key))
        .find(|items| !items.is_empty())
        .unwrap_or_default()
}

pub(crate) fn merged_string_arrays(value: &Value, keys: &[&str]) -> Vec<String> {
    let mut output = Vec::new();
    for key in keys {
        for item in get_string_array(value, key) {
            if !output.contains(&item) {
                output.push(item);
            }
        }
    }
    output
}

pub(crate) fn display_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_string())
}

pub(crate) fn display_string_array(value: &Value, key: &str) -> String {
    let items = get_string_array(value, key);
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

pub(crate) fn display_delta(value: Option<u64>, baseline: Option<u64>) -> String {
    match (value, baseline) {
        (Some(value), Some(baseline)) => {
            let delta = value as i128 - baseline as i128;
            if delta >= 0 {
                format!("+{delta}")
            } else {
                delta.to_string()
            }
        }
        _ => "unavailable".to_string(),
    }
}

pub(crate) fn supervisor_input_tokens(metrics: &Value) -> Option<u64> {
    get_u64(metrics, "supervisor_input_tokens").or_else(|| {
        metrics
            .get("codex_usage")
            .and_then(|usage| get_u64(usage, "input_tokens"))
    })
}

pub(crate) fn supervisor_output_tokens(metrics: &Value) -> Option<u64> {
    get_u64(metrics, "supervisor_output_tokens").or_else(|| {
        metrics
            .get("codex_usage")
            .and_then(|usage| get_u64(usage, "output_tokens"))
    })
}

pub(crate) fn supervisor_total_tokens(metrics: &Value) -> Option<u64> {
    get_u64(metrics, "supervisor_total_tokens")
        .or_else(|| get_u64(metrics, "codex_token_usage"))
        .or_else(|| {
            let input = supervisor_input_tokens(metrics)?;
            let output = supervisor_output_tokens(metrics)?;
            let reasoning = metrics
                .get("codex_usage")
                .and_then(|usage| get_u64(usage, "reasoning_output_tokens"))
                .or_else(|| get_u64(metrics, "supervisor_reasoning_tokens"))
                .unwrap_or(0);
            Some(input + output + reasoning)
        })
}

pub(crate) fn mixmod_provider_model(metrics: &Value) -> String {
    let provider = get_str(metrics, "opencode_provider")
        .or_else(|| {
            metrics
                .get("run_metrics")
                .and_then(|run| get_str(run, "opencode_provider"))
        })
        .unwrap_or("unknown");
    let model = get_str(metrics, "opencode_model")
        .or_else(|| {
            metrics
                .get("run_metrics")
                .and_then(|run| get_str(run, "opencode_model"))
        })
        .unwrap_or("unknown");
    format!("{provider}/{model}")
}

pub(crate) fn write_if_missing(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    atomic_write(path, bytes)
}

pub(crate) fn write_pretty_json<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    artifact: &str,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("failed to serialize {artifact} JSON for {}", path.display()))?;
    atomic_write(path, &bytes)
}

pub(crate) fn write_pretty_json_if_missing<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    artifact: &str,
) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    write_pretty_json(path, value, artifact)
}

pub(crate) fn append_jsonl(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open JSONL file {}", path.display()))?;
    let line = serde_json::to_string(value)
        .with_context(|| format!("failed to serialize JSONL value for {}", path.display()))?;
    writeln!(file, "{line}").with_context(|| format!("failed to append {}", path.display()))?;
    Ok(())
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("invalid path {}", path.display()))?;
    let tmp = path.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    {
        let mut file =
            File::create(&tmp).with_context(|| format!("failed to create {}", tmp.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("failed to write temporary file {}", tmp.display()))?;
        file.sync_all().ok();
    }
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "failed to atomically replace {} with {}",
            path.display(),
            tmp.display()
        )
    })?;
    Ok(())
}

pub(crate) fn file_len(path: &Path) -> Result<u64> {
    Ok(fs::metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path.display()))?
        .len())
}

pub(crate) fn absolutize(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

pub(crate) fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|rel| rel.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

pub(crate) fn make_run_id(prefix: &str) -> String {
    format!(
        "{}-{}-{}",
        prefix,
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        std::process::id()
    )
}

pub(crate) fn truncate_for_report(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n... truncated; inspect raw logs for full output ...");
    truncated
}
