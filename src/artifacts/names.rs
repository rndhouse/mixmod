use std::path::{Path, PathBuf};

/// Experiment task markdown artifact.
pub const TASK_MD: &str = "task.md";
/// JSON task artifact passed between Mixmod stages.
pub const TASK_JSON: &str = "task.json";
/// Worker instruction markdown artifact written before OpenCode runs.
pub const OPENCODE_INSTRUCTIONS_MD: &str = "opencode-instructions.md";
/// Worker-facing task artifact for default strategy runs.
pub const WORKER_TASK_JSON: &str = "worker-task.json";
/// Supervisor-generated worker brief artifact.
pub const WORKER_BRIEF_JSON: &str = "worker-brief.json";
/// Run receipt artifact.
pub const RECEIPT_JSON: &str = "receipt.json";
/// Receipt written when a default strategy run is blocked locally.
pub const BLOCKED_RECEIPT_JSON: &str = "blocked-receipt.json";
/// Markdown report artifact.
pub const REPORT_MD: &str = "report.md";
/// Worker session transcript artifact.
pub const SESSION_JSONL: &str = "session.jsonl";
/// Current accumulated repository patch artifact.
pub const WORKTREE_PATCH: &str = "worktree.patch";
/// Latest worker-turn patch artifact.
pub const CHANGES_PATCH: &str = "changes.patch";
/// Partial patch artifact preserved after interrupted workers.
pub const PARTIAL_PATCH: &str = "partial.patch";
/// Final patch artifact for experiment/default strategy outputs.
pub const FINAL_PATCH: &str = "final.patch";
/// Metrics artifact.
pub const METRICS_JSON: &str = "metrics.json";
/// Supervisor feedback transcript artifact.
pub const SUPERVISOR_FEEDBACK_JSONL: &str = "supervisor-feedback.jsonl";
/// Local worker verification artifact.
pub const LOCAL_VERIFICATION_JSON: &str = "local-verification.json";
/// Patch checkpoint comparison artifact.
pub const PATCH_COMPARISON: &str = "patch-comparison.json";
/// Previous accumulated worktree patch artifact.
pub const PREVIOUS_WORKTREE_PATCH: &str = "previous-worktree.patch";
/// Receipt written when Mixmod makes an accepted patch into a baseline.
pub const PATCH_BASELINE_JSON: &str = "patch-baseline.json";
/// Patch accepted into an internal baseline checkpoint.
pub const BASELINE_ACCEPTED_PATCH: &str = "baseline-accepted.patch";
/// Active worktree patch captured after an internal baseline checkpoint.
pub const BASELINE_ACTIVE_PATCH: &str = "baseline-active.patch";
/// Rollback receipt written when `revise_previous` restores a checkpoint.
pub const PATCH_ROLLBACK_JSON: &str = "patch-rollback.json";
/// Current patch saved before a `revise_previous` rollback.
pub const ROLLBACK_CURRENT_PATCH: &str = "rollback-current.patch";
/// Patch captured after a `revise_previous` rollback.
pub const ROLLBACK_RESTORED_PATCH: &str = "rollback-restored.patch";
/// Mixmod intervention audit log artifact.
pub const INTERVENTIONS_JSONL: &str = "interventions.jsonl";
/// Supervisor control event log artifact.
pub const SUPERVISOR_CONTROL_LOG: &str = "supervisor-control.jsonl";
/// Extracted worker reasoning events from structured OpenCode output.
pub const REASONING_TRACE_JSONL: &str = "reasoning-trace.jsonl";
/// Extracted worker tool-call events from structured OpenCode output.
pub const TOOL_EVENTS_JSONL: &str = "tool-events.jsonl";
/// Clean structured OpenCode stdout event stream, without raw log headers.
pub const OPENCODE_EVENTS_JSONL: &str = "opencode.events.jsonl";
/// Compact cross-turn telemetry for the supervisor's worker loop.
pub const SUPERVISION_LOOP_SUMMARY_JSON: &str = "supervision-loop-summary.json";

/// Compact artifacts that a supervisor can review for a single worker run.
pub const RUN_COMPACT_ARTIFACTS: &[&str] = &[
    RECEIPT_JSON,
    REPORT_MD,
    REASONING_TRACE_JSONL,
    TOOL_EVENTS_JSONL,
    WORKTREE_PATCH,
    CHANGES_PATCH,
    INTERVENTIONS_JSONL,
    METRICS_JSON,
];

/// Build the artifact set reviewed by the supervisor after a worker turn.
pub(crate) fn supervisor_review_artifact_paths(
    default_dir: &Path,
    worker_run_dir: &Path,
) -> Vec<PathBuf> {
    [
        TASK_JSON,
        WORKER_BRIEF_JSON,
        WORKER_TASK_JSON,
        SUPERVISION_LOOP_SUMMARY_JSON,
    ]
    .into_iter()
    .map(|name| default_dir.join(name))
    .filter(|path| path.exists())
    .chain(
        RUN_COMPACT_ARTIFACTS
            .iter()
            .map(|name| worker_run_dir.join(name)),
    )
    .collect()
}

/// Supervisor-visible worker-turn artifacts, including checkpoint artifacts.
pub const CODEX_REVIEW_ARTIFACTS: &[&str] = &[
    RECEIPT_JSON,
    REPORT_MD,
    SUPERVISION_LOOP_SUMMARY_JSON,
    WORKTREE_PATCH,
    CHANGES_PATCH,
    REASONING_TRACE_JSONL,
    TOOL_EVENTS_JSONL,
    INTERVENTIONS_JSONL,
    METRICS_JSON,
    PATCH_COMPARISON,
    PREVIOUS_WORKTREE_PATCH,
    PATCH_BASELINE_JSON,
    BASELINE_ACCEPTED_PATCH,
    BASELINE_ACTIVE_PATCH,
    PATCH_ROLLBACK_JSON,
    ROLLBACK_CURRENT_PATCH,
    ROLLBACK_RESTORED_PATCH,
];

/// Artifacts copied or size-counted from worker/default strategy run dirs.
pub const WORKER_RUN_ARTIFACTS: &[&str] = &[
    WORKER_BRIEF_JSON,
    WORKER_TASK_JSON,
    RECEIPT_JSON,
    TASK_JSON,
    OPENCODE_INSTRUCTIONS_MD,
    REPORT_MD,
    SESSION_JSONL,
    REASONING_TRACE_JSONL,
    TOOL_EVENTS_JSONL,
    WORKTREE_PATCH,
    CHANGES_PATCH,
    INTERVENTIONS_JSONL,
    PATCH_COMPARISON,
    PREVIOUS_WORKTREE_PATCH,
    PATCH_BASELINE_JSON,
    BASELINE_ACCEPTED_PATCH,
    BASELINE_ACTIVE_PATCH,
    PATCH_ROLLBACK_JSON,
    ROLLBACK_CURRENT_PATCH,
    ROLLBACK_RESTORED_PATCH,
    PARTIAL_PATCH,
    SUPERVISION_LOOP_SUMMARY_JSON,
    METRICS_JSON,
    SUPERVISOR_FEEDBACK_JSONL,
    FINAL_PATCH,
    LOCAL_VERIFICATION_JSON,
    SUPERVISOR_CONTROL_LOG,
];

/// Returns true for static artifact file names managed by Mixmod.
pub(crate) fn is_static_mixmod_artifact_name(file_name: &str) -> bool {
    WORKER_RUN_ARTIFACTS.contains(&file_name)
        || file_name == TASK_MD
        || file_name == BLOCKED_RECEIPT_JSON
}
