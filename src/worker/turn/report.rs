use super::context::WorkerContextSignals;
use crate::*;

pub(crate) fn build_worker_turn_summary(
    status: &str,
    mode: DelegationMode,
    output: &AgentOutput,
    stats: &PatchStats,
    worktree_stats: &PatchStats,
) -> String {
    match status {
        "success" => format!(
            "Worker completed {mode}; {} file(s) and {} line(s) changed.",
            stats.files.len(),
            stats.changed_line_count
        ),
        "needs_supervisor"
            if output.timed_out || output.idle_timed_out || output.interrupted_by_supervisor =>
        {
            let reason = if output.interrupted_by_supervisor {
                "supervisor control interrupt"
            } else if output.timed_out {
                "worker timeout"
            } else {
                "idle timeout"
            };
            format!(
                "Worker stopped for {mode} after {reason}; {} file(s) and {} line(s) were captured for supervisor recovery.",
                stats.files.len(),
                stats.changed_line_count
            )
        }
        "needs_supervisor" if !stats.files.is_empty() => format!(
            "Worker completed {mode} with {} file(s) and {} line(s) changed; supervisor review needed.",
            stats.files.len(),
            stats.changed_line_count
        ),
        "needs_supervisor" if !worktree_stats.files.is_empty() => format!(
            "Worker completed {mode} with no new delta, but current worktree patch has {} file(s) and {} line(s) changed; supervisor review needed.",
            worktree_stats.files.len(),
            worktree_stats.changed_line_count
        ),
        "needs_supervisor" => {
            format!("Worker completed {mode} but no patch was captured; supervisor review needed.")
        }
        _ => format!(
            "Worker failed or could not be started for {mode}; exit status {:?}, stderr {} bytes.",
            output.exit_status,
            output.stderr.len()
        ),
    }
}

pub(super) struct WorkerTurnReportInput<'a> {
    pub(super) status: &'a str,
    pub(super) mode: DelegationMode,
    pub(super) summary: &'a str,
    pub(super) task: &'a TaskSpec,
    pub(super) output: &'a AgentOutput,
    pub(super) stats: &'a PatchStats,
    pub(super) worktree_stats: &'a PatchStats,
    pub(super) context_overflow: &'a WorkerContextSignals,
    pub(super) worker_session_token_peak: Option<u64>,
    pub(super) notes: &'a [String],
    pub(super) root: &'a Path,
    pub(super) out_dir: &'a Path,
}

pub(super) fn build_worker_turn_report(input: WorkerTurnReportInput<'_>) -> String {
    let WorkerTurnReportInput {
        status,
        mode,
        summary,
        task,
        output,
        stats,
        worktree_stats,
        context_overflow,
        worker_session_token_peak,
        notes,
        root,
        out_dir,
    } = input;
    let reasoning_trace_path = out_dir.join(REASONING_TRACE_JSONL);
    let reasoning_trace = fs::read_to_string(&reasoning_trace_path).unwrap_or_default();
    let reasoning_event_count = reasoning_trace
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let tool_events_path = out_dir.join(TOOL_EVENTS_JSONL);
    let tool_events = fs::read_to_string(&tool_events_path).unwrap_or_default();
    let tool_event_count = tool_events
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let files = if stats.files.is_empty() {
        "- none captured".to_string()
    } else {
        stats
            .files
            .iter()
            .map(|file| format!("- `{file}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let worker_check_guidance = if task.tests.is_empty() {
        "- none specified in task metadata".to_string()
    } else {
        task.tests
            .iter()
            .map(|test| format!("- `{test}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let notes = notes
        .iter()
        .map(|note| format!("- {note}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"# Mixmod Worker Turn Report

## Summary

- Status: {status}
- Mode: {mode}
- Task: {task_title}
- Result: {summary}
- Worker backend: {worker_backend}
- Worker exit status: {exit_status}
- Worker session label: {session_label}
- Worker session id: {session_id}
- Worker resumed session id: {resume_session_id}
- Worker session reused: {session_reused}
- Interrupted by supervisor control: {interrupted_by_supervisor}
- Supervisor control action: {supervisor_control_action}
- Worker timed out: {timed_out}
- Worker idle timed out: {idle_timed_out}
- Heartbeats: {heartbeat_count}
- Worker context overflow events: {context_overflow_count}
- Last context overflow: {context_overflow_last}
- Worker session token peak: {worker_session_token_peak}

## Changed Files

{files}

Changed lines: {changed_lines} ({added} added, {removed} removed)

## Current Worktree Patch

- Files: {worktree_files}
- Changed lines: {worktree_changed_lines} ({worktree_added} added, {worktree_removed} removed)
- Artifact: `{worktree_patch}`

## Checks

Mixmod does not execute project test commands directly in worker runs.
Worker-run checks, if any, are captured in worker tool events and stdout/stderr.
They remain untrusted until supervisor review or an external evaluator confirms the patch.

Worker-facing check guidance from task metadata:

{worker_check_guidance}

## Review Signals

- Artifact: `{review_signals}`

## Conditional Diagnostics

Detailed artifacts are for targeted debugging or evidence checks; normal
supervisor review should not open them unless review-signals.json or the
task evidence makes them relevant.

- Worker reasoning events: {reasoning_event_count}; `{reasoning_trace_artifact}`
- Worker tool events: {tool_event_count}; `{tool_events_artifact}`
- Worker stdout bytes: {stdout_bytes}
- Worker stderr bytes: {stderr_bytes}

## Core Review Artifact Paths

- `{receipt}`
- `{report}`
- `{review_signals}`
- `{patch}`

Diagnostic artifacts are available under `{out_dir}` when needed.
Worktree patch: `{worktree_patch}`
Interventions: `{interventions}`
Metrics: `{metrics}`
Heartbeat log: `{heartbeat}`

## Notes

{notes}
"#,
        status = status,
        mode = mode,
        task_title = task.title,
        summary = summary,
        worker_backend = output.backend.as_str(),
        exit_status = worker_turn_exit_status_label(output),
        session_label = output.session_label.as_deref().unwrap_or("unavailable"),
        session_id = output.session_id.as_deref().unwrap_or("unavailable"),
        resume_session_id = output.resume_session_id.as_deref().unwrap_or("none"),
        session_reused = yes_no(output.session_reused),
        interrupted_by_supervisor = yes_no(output.interrupted_by_supervisor),
        supervisor_control_action = output
            .supervisor_control_action
            .as_deref()
            .unwrap_or("none"),
        timed_out = yes_no(output.timed_out),
        idle_timed_out = yes_no(output.idle_timed_out),
        heartbeat_count = output.heartbeat_count,
        context_overflow_count = context_overflow.context_overflow_count,
        context_overflow_last = context_overflow
            .context_overflow_last_message
            .as_deref()
            .unwrap_or("none"),
        worker_session_token_peak = worker_session_token_peak
            .map(|tokens| tokens.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        files = files,
        changed_lines = stats.changed_line_count,
        added = stats.added_lines,
        removed = stats.removed_lines,
        worktree_files = worktree_stats.files.len(),
        worktree_changed_lines = worktree_stats.changed_line_count,
        worktree_added = worktree_stats.added_lines,
        worktree_removed = worktree_stats.removed_lines,
        worker_check_guidance = worker_check_guidance,
        reasoning_event_count = reasoning_event_count,
        reasoning_trace_artifact = display_path(root, &reasoning_trace_path),
        tool_event_count = tool_event_count,
        tool_events_artifact = display_path(root, &tool_events_path),
        stdout_bytes = output.stdout.len(),
        stderr_bytes = output.stderr.len(),
        receipt = display_path(root, &out_dir.join(RECEIPT_JSON)),
        report = display_path(root, &out_dir.join(REPORT_MD)),
        review_signals = display_path(root, &out_dir.join(REVIEW_SIGNALS_JSON)),
        worktree_patch = display_path(root, &out_dir.join(WORKTREE_PATCH)),
        patch = display_path(root, &out_dir.join(CHANGES_PATCH)),
        interventions = display_path(root, &out_dir.join(INTERVENTIONS_JSONL)),
        metrics = display_path(root, &out_dir.join(METRICS_JSON)),
        heartbeat = display_path(root, &out_dir.join("logs/heartbeat.jsonl")),
        out_dir = display_path(root, out_dir),
        notes = notes,
    )
}

pub(crate) fn worker_turn_exit_status_label(output: &AgentOutput) -> String {
    if let Some(code) = output.exit_status {
        return code.to_string();
    }
    if output.interrupted_by_supervisor {
        return "interrupted-by-supervisor".to_string();
    }
    if output.timed_out {
        return "worker-timeout".to_string();
    }
    if output.idle_timed_out {
        return "idle-timeout".to_string();
    }
    "spawn-failed".to_string()
}
