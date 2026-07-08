use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use crate::harness::{AgentRequest, LiveWorkerSnapshot};
use crate::{
    diff_without_unchanged_blocks, get_str, git_diff_with_untracked, patch_stats,
    truncate_for_report, worker_session_token_peak,
};

use crate::harness::opencode::control::tail_text;

pub(super) struct LiveWorkerSnapshotInput<'a> {
    pub(super) request: &'a AgentRequest,
    pub(super) out_dir: &'a Path,
    pub(super) stdout_path: &'a Path,
    pub(super) stderr_path: &'a Path,
    pub(super) baseline_diff: &'a str,
    pub(super) segment_stdout_start: u64,
    pub(super) start: Instant,
    pub(super) segment_started_instant: Instant,
    pub(super) segment_index: u64,
    pub(super) segment_action: &'a str,
    pub(super) segment_worker_mode: &'a str,
    pub(super) segment_label: &'a str,
    pub(super) segment_resume_session_id: Option<&'a str>,
    pub(super) stdout_bytes: u64,
    pub(super) stderr_bytes: u64,
    pub(super) last_output_age: u64,
    pub(super) gpu_activity_observed: bool,
    pub(super) backend_activity_observed: bool,
}

pub(super) fn build_live_worker_snapshot(
    input: LiveWorkerSnapshotInput<'_>,
) -> Result<LiveWorkerSnapshot> {
    let current_diff = git_diff_with_untracked(&input.request.root).unwrap_or_default();
    let new_delta = diff_without_unchanged_blocks(&current_diff, input.baseline_diff);
    let stats = patch_stats(&new_delta);
    let stdout = fs::read(input.stdout_path).unwrap_or_default();
    let segment_start = (input.segment_stdout_start as usize).min(stdout.len());
    let segment_stdout = &stdout[segment_start..];
    let tool_activity = summarize_live_tool_activity(segment_stdout);
    Ok(LiveWorkerSnapshot {
        out_dir: input.out_dir.to_string_lossy().to_string(),
        mode: input.request.mode.to_string(),
        task_path: input.request.task_path.to_string_lossy().to_string(),
        session_label: input.segment_label.to_string(),
        resume_session_id: input.segment_resume_session_id.map(ToOwned::to_owned),
        opencode_segment: input.segment_index,
        segment_action: input.segment_action.to_string(),
        segment_worker_mode: input.segment_worker_mode.to_string(),
        worker_instruction_excerpt: truncate_for_report(&input.request.instruction, 6000),
        live_control_check_index: 0,
        live_control_check_limit: 0,
        elapsed_ms: millis_u64(input.start.elapsed()),
        segment_elapsed_ms: millis_u64(input.segment_started_instant.elapsed()),
        stdout_bytes: input.stdout_bytes,
        stderr_bytes: input.stderr_bytes,
        last_output_age_ms: input.last_output_age,
        gpu_activity_observed: input.gpu_activity_observed,
        backend_activity_observed: input.backend_activity_observed,
        new_delta_bytes: new_delta.len() as u64,
        new_delta_files: stats.files,
        new_delta_changed_line_count: stats.changed_line_count,
        context_overflow_count: count_context_overflow(segment_stdout),
        worker_session_token_peak: worker_session_token_peak(segment_stdout),
        repeated_read_signature: tool_activity.repeated_read_signature,
        repeated_read_count: tool_activity.repeated_read_count,
        recent_tool_events: tool_activity.recent_tool_events,
        stdout_tail: tail_text(input.stdout_path, 6000),
        stderr_tail: tail_text(input.stderr_path, 2000),
    })
}

fn millis_u64(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[derive(Debug, Default)]
struct LiveToolActivity {
    repeated_read_signature: Option<String>,
    repeated_read_count: u64,
    recent_tool_events: Vec<String>,
}

fn summarize_live_tool_activity(stdout: &[u8]) -> LiveToolActivity {
    let mut counts = BTreeMap::<String, u64>::new();
    let mut recent = Vec::new();
    let text = String::from_utf8_lossy(stdout);
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if get_str(&event, "type") != Some("tool_use") {
            continue;
        }
        let tool = event
            .pointer("/part/tool")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let input = tool_input_summary(&event);
        let status = event
            .pointer("/part/state/status")
            .and_then(Value::as_str)
            .unwrap_or("");
        let event_summary = if status.is_empty() {
            format!("{tool}: {input}")
        } else {
            format!("{tool}: {input} => {status}")
        };
        recent.push(event_summary);
        if recent.len() > 20 {
            recent.remove(0);
        }
        if is_read_like_tool(tool) {
            *counts.entry(format!("{tool}: {input}")).or_default() += 1;
        }
    }

    let (repeated_read_signature, repeated_read_count) = counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(signature, count)| (Some(signature), count))
        .unwrap_or((None, 0));

    LiveToolActivity {
        repeated_read_signature,
        repeated_read_count,
        recent_tool_events: recent,
    }
}

fn tool_input_summary(event: &Value) -> String {
    let value = event
        .pointer("/part/state/input/filePath")
        .or_else(|| event.pointer("/part/state/input/pattern"))
        .or_else(|| event.pointer("/part/state/input/path"))
        .or_else(|| event.pointer("/part/state/input/command"))
        .and_then(Value::as_str)
        .unwrap_or("");
    truncate_tool_input(value)
}

fn truncate_tool_input(value: &str) -> String {
    let value = value.trim().replace('\n', " ");
    if value.chars().count() <= 180 {
        return value;
    }
    let mut truncated = value.chars().take(180).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn is_read_like_tool(tool: &str) -> bool {
    matches!(tool, "read" | "grep" | "glob" | "list")
}

fn count_context_overflow(stdout: &[u8]) -> u64 {
    let text = String::from_utf8_lossy(stdout);
    text.lines()
        .filter(|line| {
            line.contains("ContextOverflowError")
                || line.contains("exceeds the available context size")
        })
        .count() as u64
}
