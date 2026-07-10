//! Agent harnesses used by Mixmod roles.
//!
//! A harness is the adapter that runs an external coding agent backend. Mixmod
//! strategy code decides whether an agent is acting as supervisor or worker;
//! harness code only executes the requested backend turn and reports what
//! happened.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    DelegationMode, MixmodConfig, SupervisorControlEvent, WorkerBackend, WorkerBackendTelemetry,
};

pub(crate) mod codex;
pub(crate) mod opencode;

/// Role an agent turn is serving in the Mixmod loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRole {
    /// Review, plan, or decide without directly editing the repository.
    Supervisor,
    /// Inspect, edit, and test the repository.
    Worker,
}

/// Backend implementation used for an agent turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentBackend {
    /// Codex app-server or Codex CLI based execution.
    Codex,
    /// OpenCode based execution.
    OpenCode,
}

impl AgentBackend {
    /// Stable metric label for this backend.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

/// Session behavior requested by the strategy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentSessionMode {
    /// Start a new backend session.
    New,
    /// Continue the named backend session.
    Continue { session_id: String },
    /// Start a fresh backend session on the current worktree.
    ContextFocus,
}

/// Compact live state from a running worker turn.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LiveWorkerSnapshot {
    /// Worker run artifact directory.
    pub out_dir: String,
    /// Delegation mode for the current worker turn.
    pub mode: String,
    /// Worker task JSON path.
    pub task_path: String,
    /// OpenCode session label for this worker run.
    pub session_label: String,
    /// OpenCode session id being resumed, when any.
    pub resume_session_id: Option<String>,
    /// Current OpenCode segment index.
    pub opencode_segment: u64,
    /// Segment action such as initial, interrupt_continue, or context focus.
    pub segment_action: String,
    /// Segment worker mode such as initial, continue, or context_focus.
    pub segment_worker_mode: String,
    /// Bounded excerpt of the instruction currently given to the worker.
    pub worker_instruction_excerpt: String,
    /// Current live supervisor check index within this worker run.
    pub live_control_check_index: u64,
    /// Maximum live supervisor checks allowed for this worker run.
    pub live_control_check_limit: u64,
    /// Total worker-run elapsed time in milliseconds.
    pub elapsed_ms: u64,
    /// Current segment elapsed time in milliseconds.
    pub segment_elapsed_ms: u64,
    /// Captured worker stdout byte count.
    pub stdout_bytes: u64,
    /// Captured worker stderr byte count.
    pub stderr_bytes: u64,
    /// Milliseconds since the worker last emitted stdout or stderr.
    pub last_output_age_ms: u64,
    /// Current repository diff size relative to the start of this worker run.
    pub new_delta_bytes: u64,
    /// Files touched by the current repository diff relative to this worker run.
    pub new_delta_files: Vec<String>,
    /// Changed-line count in the current diff relative to this worker run.
    pub new_delta_changed_line_count: usize,
    /// Number of context-overflow events seen in current worker stdout.
    pub context_overflow_count: u64,
    /// Peak total-token count reported by structured worker stdout.
    pub worker_session_token_peak: Option<u64>,
    /// Raw backend telemetry from the worker inference server, when available.
    pub worker_backend_telemetry: Option<WorkerBackendTelemetry>,
    /// Live-updated worker stdout log path.
    pub stdout_log_path: String,
    /// Live-updated worker stderr log path.
    pub stderr_log_path: String,
    /// Live-updated JSONL artifact containing structured worker tool-call events.
    pub tool_events_path: String,
}

/// Optional strategy hook that can steer a live worker turn.
pub trait SupervisorAdvisor: Send + Sync {
    /// Return a supervisor control command for the harness, or `None` to wait.
    fn advise(&self, snapshot: &LiveWorkerSnapshot) -> Result<Option<Value>>;
}

/// Request passed from Mixmod strategy/run code to an agent harness.
pub struct AgentRequest {
    pub root: PathBuf,
    pub mode: DelegationMode,
    pub task_path: PathBuf,
    pub out_dir: PathBuf,
    pub instruction_path: PathBuf,
    pub instruction: String,
    pub session_id: String,
    pub resume_session_id: Option<String>,
    pub require_local: bool,
    pub supervisor_advisor: Option<Arc<dyn SupervisorAdvisor>>,
}

impl fmt::Debug for AgentRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentRequest")
            .field("root", &self.root)
            .field("mode", &self.mode)
            .field("task_path", &self.task_path)
            .field("out_dir", &self.out_dir)
            .field("instruction_path", &self.instruction_path)
            .field("instruction", &self.instruction)
            .field("session_id", &self.session_id)
            .field("resume_session_id", &self.resume_session_id)
            .field("require_local", &self.require_local)
            .field(
                "supervisor_advisor",
                &self.supervisor_advisor.as_ref().map(|_| "configured"),
            )
            .finish()
    }
}

impl AgentRequest {
    /// Return the neutral session mode implied by this request.
    pub fn session_mode(&self) -> AgentSessionMode {
        match self.resume_session_id.as_ref() {
            Some(session_id) => AgentSessionMode::Continue {
                session_id: session_id.clone(),
            },
            None => AgentSessionMode::New,
        }
    }
}

/// Output returned by an agent harness after one Mixmod run segment.
#[derive(Debug)]
pub struct AgentOutput {
    pub backend: AgentBackend,
    pub command_for_metrics: Vec<String>,
    pub segments: Vec<Value>,
    pub exit_status: Option<i32>,
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub model_arg: Option<String>,
    pub session_label: Option<String>,
    pub session_id: Option<String>,
    pub resume_session_id: Option<String>,
    pub session_reused: bool,
    pub interrupted_by_supervisor: bool,
    pub supervisor_control_action: Option<String>,
    pub supervisor_control_events: Vec<SupervisorControlEvent>,
    pub timed_out: bool,
    pub idle_timed_out: bool,
    pub heartbeat_count: u64,
    pub require_local: bool,
    pub local_inference_verified: bool,
    pub gpu_activity_observed: bool,
    pub backend_activity_observed: bool,
    pub verification_notes: Vec<String>,
}

/// Adapter interface for running an agent backend.
pub trait AgentHarness {
    fn run(&self, request: &AgentRequest) -> Result<AgentOutput>;
}

/// Compatibility alias for the existing worker request name.
pub type OpenCodeRequest = AgentRequest;

/// Compatibility alias for the existing worker output name.
pub type OpenCodeOutput = AgentOutput;

/// Compatibility trait for existing OpenCode runner call sites.
pub trait OpenCodeRunner: AgentHarness {}

impl<T: AgentHarness + ?Sized> OpenCodeRunner for T {}

/// Build the worker harness configured for this run.
pub fn worker_harness_for_config(config: MixmodConfig) -> Box<dyn AgentHarness> {
    match config.worker.backend {
        WorkerBackend::OpenCode => Box::new(opencode::ShellOpenCodeRunner::new(config)),
        WorkerBackend::Codex => Box::new(codex::ShellCodexRunner::new(config)),
    }
}
