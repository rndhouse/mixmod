use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{Value, json};

use crate::*;

use super::types::SupervisorContextTelemetry;

const REVIEW_PACKET_MAX_CHARS: usize = 64_000;
const REVIEW_PACKET_DEFAULT_ARTIFACT_CHARS: usize = 6_000;
const REVIEW_PACKET_PATCH_CHARS: usize = 20_000;
const REVIEW_PACKET_LOOP_SUMMARY_CHARS: usize = 12_000;
const REVIEW_PACKET_SIGNALS_CHARS: usize = 10_000;
const REVIEW_PACKET_TASK_CHARS: usize = 5_000;
const TRUNCATION_MARKER: &str = "\n... truncated by Mixmod review packet budget ...";

/// Bounded artifact packet supplied to a spin-out supervisor reviewer.
pub(crate) struct SupervisorReviewPacket {
    /// JSON packet embedded into the reviewer prompt.
    pub(crate) value: Value,
    /// Total included text characters across artifact excerpts.
    pub(crate) included_chars: usize,
    /// Artifact display paths whose content was truncated.
    pub(crate) truncated_artifacts: Vec<String>,
}

/// Build the bounded context packet for a fresh supervisor review session.
pub(crate) fn build_supervisor_review_packet(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
    context_telemetry: &SupervisorContextTelemetry,
) -> Result<SupervisorReviewPacket> {
    let mut remaining_chars = REVIEW_PACKET_MAX_CHARS;
    let mut included_chars = 0_usize;
    let mut artifacts = Vec::new();
    let mut missing_artifacts = Vec::new();
    let mut truncated_artifacts = Vec::new();

    for path in artifact_paths {
        let display_path = display_path(work_dir, path);
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("artifact");
        let limit = artifact_char_limit(file_name).min(remaining_chars);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                missing_artifacts.push(json!({
                    "path": display_path,
                    "file_name": file_name,
                    "error": error.to_string()
                }));
                continue;
            }
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let original_chars = text.chars().count();
        let (content, truncated) = truncate_packet_text(&text, limit);
        let content_chars = content.chars().count();
        remaining_chars = remaining_chars.saturating_sub(content_chars);
        included_chars += content_chars;
        if truncated {
            truncated_artifacts.push(display_path.clone());
        }

        let patch_stats = if file_name.ends_with(".patch") {
            json!(patch_stats(&text))
        } else {
            Value::Null
        };
        artifacts.push(json!({
            "path": display_path,
            "file_name": file_name,
            "bytes": bytes.len(),
            "chars": original_chars,
            "included_chars": content_chars,
            "truncated": truncated,
            "patch_stats": patch_stats,
            "content": content
        }));
    }

    let value = json!({
        "kind": "mixmod-spin-out-supervisor-review-packet",
        "schema_version": 1,
        "generated_at": Utc::now().to_rfc3339(),
        "working_repo": work_dir.display().to_string(),
        "instruction": instruction,
        "context_telemetry": context_telemetry.to_prompt_json(),
        "limits": {
            "max_chars": REVIEW_PACKET_MAX_CHARS,
            "default_artifact_chars": REVIEW_PACKET_DEFAULT_ARTIFACT_CHARS,
            "patch_chars": REVIEW_PACKET_PATCH_CHARS,
            "loop_summary_chars": REVIEW_PACKET_LOOP_SUMMARY_CHARS,
            "review_signals_chars": REVIEW_PACKET_SIGNALS_CHARS,
            "task_chars": REVIEW_PACKET_TASK_CHARS
        },
        "included_chars": included_chars,
        "truncated_artifacts": truncated_artifacts,
        "missing_artifacts": missing_artifacts,
        "artifacts": artifacts
    });

    Ok(SupervisorReviewPacket {
        value,
        included_chars,
        truncated_artifacts,
    })
}

/// Persist a review packet next to the supervisor prompt artifacts.
pub(crate) fn write_supervisor_review_packet(
    artifact_dir: &Path,
    label: &str,
    packet: &SupervisorReviewPacket,
) -> Result<PathBuf> {
    let packet_path = artifact_dir.join(format!("{label}-supervisor-review-packet.json"));
    write_pretty_json(&packet_path, &packet.value, "supervisor review packet")
        .with_context(|| format!("failed to write {}", packet_path.display()))?;
    Ok(packet_path)
}

fn artifact_char_limit(file_name: &str) -> usize {
    match file_name {
        CHANGES_PATCH
        | WORKTREE_PATCH
        | PREVIOUS_WORKTREE_PATCH
        | BASELINE_ACCEPTED_PATCH
        | BASELINE_ACTIVE_PATCH
        | ROLLBACK_CURRENT_PATCH
        | ROLLBACK_RESTORED_PATCH => REVIEW_PACKET_PATCH_CHARS,
        SUPERVISION_LOOP_SUMMARY_JSON => REVIEW_PACKET_LOOP_SUMMARY_CHARS,
        REVIEW_SIGNALS_JSON => REVIEW_PACKET_SIGNALS_CHARS,
        TASK_JSON | WORKER_TASK_JSON | WORKER_BRIEF_JSON => REVIEW_PACKET_TASK_CHARS,
        _ => REVIEW_PACKET_DEFAULT_ARTIFACT_CHARS,
    }
}

fn truncate_packet_text(value: &str, max_chars: usize) -> (String, bool) {
    let original_chars = value.chars().count();
    if original_chars <= max_chars {
        return (value.to_string(), false);
    }
    if max_chars == 0 {
        return (String::new(), true);
    }
    let marker_chars = TRUNCATION_MARKER.chars().count();
    if max_chars <= marker_chars {
        return (value.chars().take(max_chars).collect(), true);
    }
    let head_chars = max_chars - marker_chars;
    let mut truncated = value.chars().take(head_chars).collect::<String>();
    truncated.push_str(TRUNCATION_MARKER);
    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn review_packet_truncates_large_patch_artifacts() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let patch = root.join(CHANGES_PATCH);
        atomic_write(
            &patch,
            format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n{}",
                "+added line\n".repeat(10_000)
            )
            .as_bytes(),
        )
        .unwrap();

        let packet = build_supervisor_review_packet(
            root,
            &[patch],
            "decide",
            &SupervisorContextTelemetry::default(),
        )
        .unwrap();

        assert!(packet.included_chars <= REVIEW_PACKET_MAX_CHARS);
        assert_eq!(packet.truncated_artifacts.len(), 1);
        let artifact = packet
            .value
            .get("artifacts")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .unwrap();
        assert_eq!(
            artifact.get("truncated").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            artifact
                .get("patch_stats")
                .and_then(|stats| stats.get("changed_line_count"))
                .and_then(Value::as_u64),
            Some(10_000)
        );
    }
}
