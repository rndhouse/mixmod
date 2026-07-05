use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
/// Mixmod intervention audit log artifact.
pub const INTERVENTIONS_JSONL: &str = "interventions.jsonl";
/// Supervisor control event log artifact.
pub const SUPERVISOR_CONTROL_LOG: &str = "supervisor-control.jsonl";

/// Compact artifacts that a supervisor can review for a single worker run.
pub const RUN_COMPACT_ARTIFACTS: &[&str] = &[
    RECEIPT_JSON,
    REPORT_MD,
    WORKTREE_PATCH,
    CHANGES_PATCH,
    INTERVENTIONS_JSONL,
    METRICS_JSON,
];

/// Supervisor-visible worker-run artifacts, including checkpoint artifacts.
pub const CODEX_REVIEW_ARTIFACTS: &[&str] = &[
    RECEIPT_JSON,
    REPORT_MD,
    WORKTREE_PATCH,
    CHANGES_PATCH,
    INTERVENTIONS_JSONL,
    METRICS_JSON,
    PATCH_COMPARISON,
    PREVIOUS_WORKTREE_PATCH,
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
    WORKTREE_PATCH,
    CHANGES_PATCH,
    INTERVENTIONS_JSONL,
    PATCH_COMPARISON,
    PREVIOUS_WORKTREE_PATCH,
    PARTIAL_PATCH,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Receipt {
    pub run_id: String,
    pub status: String,
    pub mode: String,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub report: String,
    pub patch: String,
    pub worktree_patch: String,
    pub session: String,
    pub interventions: String,
    pub metrics: String,
    pub logs: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RunMetrics {
    pub start_timestamp: String,
    pub end_timestamp: String,
    pub wall_clock_ms: u128,
    pub worker_backend: String,
    pub opencode_command: Vec<String>,
    pub opencode_segments: Vec<Value>,
    pub opencode_exit_status: Option<i32>,
    pub opencode_provider: Option<String>,
    pub opencode_model: Option<String>,
    pub opencode_model_arg: Option<String>,
    pub opencode_session_label: Option<String>,
    pub opencode_session_id: Option<String>,
    pub opencode_resume_session_id: Option<String>,
    pub worker_session_reused: bool,
    pub interrupted_by_supervisor: bool,
    pub supervisor_control_action: Option<String>,
    pub supervisor_control_events: Vec<SupervisorControlEvent>,
    pub opencode_timed_out: bool,
    pub opencode_idle_timed_out: bool,
    pub heartbeat_count: u64,
    pub expect_patch: bool,
    pub intervention_count: usize,
    pub intervention_kinds: Vec<String>,
    pub intervention_artifact: String,
    pub empty_patch_followup_triggered: bool,
    pub empty_patch_followup_performed: bool,
    pub empty_patch_followup_patch_created: bool,
    pub empty_patch_followup_reason: Option<String>,
    pub empty_patch_followup_run_dir: Option<String>,
    pub revision_delta_expected: bool,
    pub revision_delta_bytes: u64,
    pub revision_noop_followup_triggered: bool,
    pub revision_noop_followup_performed: bool,
    pub revision_noop_followup_patch_created: bool,
    pub revision_noop_followup_reason: Option<String>,
    pub revision_noop_followup_run_dir: Option<String>,
    pub require_local: bool,
    pub local_inference_verified: bool,
    pub gpu_activity_observed: bool,
    pub backend_activity_observed: bool,
    pub verification_notes: Vec<String>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub report_bytes: u64,
    pub patch_bytes: u64,
    pub worktree_patch_bytes: u64,
    pub session_bytes: u64,
    pub changed_file_count: usize,
    pub changed_line_count: usize,
    pub codex_token_usage: Option<u64>,
    pub approximate_codex_input_bytes: Option<u64>,
    pub approximate_codex_output_bytes: Option<u64>,
    pub artifact_files_read_by_codex: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SupervisorControlCommand {
    pub timestamp: String,
    pub action: String,
    pub worker_mode: String,
    pub message_to_worker: String,
    pub focus_files: Vec<String>,
    pub required_checks: Vec<String>,
    pub risk: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SupervisorControlEvent {
    pub timestamp: String,
    pub action: String,
    pub worker_mode: String,
    pub message_to_worker: String,
    pub focus_files: Vec<String>,
    pub required_checks: Vec<String>,
    pub risk: String,
    pub control: Value,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub last_output_age_ms: u64,
    pub opencode_segment: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SupervisorFeedback {
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub worker_mode: Option<String>,
    #[serde(default)]
    pub patch_decision: Option<String>,
    #[serde(default)]
    pub message_to_worker: Option<String>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub focus_files: Vec<String>,
    #[serde(default)]
    pub required_checks: Vec<String>,
    #[serde(default)]
    pub risk: Option<String>,
}

impl SupervisorFeedback {
    pub fn from_value(value: &Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WorkerBrief {
    #[serde(default)]
    pub handoff: Option<String>,
    #[serde(default)]
    pub expect_patch: Option<bool>,
    #[serde(default)]
    pub worker_turn_shape: Option<String>,
    #[serde(default)]
    pub turn_goal: Option<String>,
    #[serde(default)]
    pub message_to_worker: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub supplement: Option<String>,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub focus_files: Vec<String>,
    #[serde(default)]
    pub target_files: Vec<String>,
    #[serde(default)]
    pub tests: Vec<String>,
    #[serde(default)]
    pub required_tests: Vec<String>,
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub must_check: Vec<String>,
    #[serde(default)]
    pub required_checks: Vec<String>,
    #[serde(default)]
    pub acceptance_checks: Vec<String>,
    #[serde(default)]
    pub avoid: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub implementation_steps: Vec<String>,
    #[serde(default)]
    pub exact_edits: Vec<String>,
    #[serde(default)]
    pub forbidden_actions: Vec<String>,
    #[serde(default)]
    pub deferred_checks: Vec<String>,
    #[serde(default)]
    pub defer_checks_until_patch_exists: Option<bool>,
    #[serde(default)]
    pub completion_gate: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
}

impl WorkerBrief {
    pub fn from_value(value: &Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DefaultStrategyMetrics {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
    pub final_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_receipt: Option<Receipt>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodexOnlyMetrics {
    pub kind: String,
    pub recorded_at: String,
    pub final_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug)]
pub struct ExperimentReportInputs {
    pub codex_metrics: Value,
    pub default_metrics: Value,
    pub default_source: String,
    pub default_metrics_path: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PatchStats {
    pub files: Vec<String>,
    pub changed_line_count: usize,
    pub added_lines: usize,
    pub removed_lines: usize,
}
