use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
    pub tests: String,
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
    pub empty_patch_followup_triggered: bool,
    pub empty_patch_followup_performed: bool,
    pub empty_patch_followup_patch_created: bool,
    pub empty_patch_followup_reason: Option<String>,
    pub empty_patch_followup_run_dir: Option<String>,
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
    pub test_status: String,
    pub test_commands: Vec<String>,
    pub test_results: Vec<TestCommandResult>,
    pub changed_file_count: usize,
    pub changed_line_count: usize,
    pub codex_token_usage: Option<u64>,
    pub approximate_codex_input_bytes: Option<u64>,
    pub approximate_codex_output_bytes: Option<u64>,
    pub artifact_files_read_by_codex: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestCommandResult {
    pub command: String,
    pub status: String,
    pub exit_status: Option<i32>,
    pub wall_clock_ms: u128,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_log: String,
    pub stderr_log: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestArtifact {
    pub status: String,
    pub requested: Vec<String>,
    pub observed: Vec<TestCommandResult>,
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
pub struct FrontierFeedback {
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

impl FrontierFeedback {
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
    pub test_status: String,
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
