use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_turn_shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exact_edits: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edit_plan: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deferred_checks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_checks_until_patch_exists: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_gate: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_actions: Vec<String>,
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
    #[serde(default)]
    pub worker_turn_shape: Option<String>,
    #[serde(default)]
    pub worker_role: Option<String>,
    #[serde(default)]
    pub turn_goal: Option<String>,
    #[serde(default)]
    pub exact_edits: Vec<String>,
    #[serde(default)]
    pub edit_plan: Vec<String>,
    #[serde(default)]
    pub deferred_checks: Vec<String>,
    #[serde(default)]
    pub defer_checks_until_patch_exists: Option<bool>,
    #[serde(default)]
    pub completion_gate: Option<String>,
    #[serde(default)]
    pub forbidden_actions: Vec<String>,
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
    pub worker_role: Option<String>,
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
    pub edit_plan: Vec<String>,
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
