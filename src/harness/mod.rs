//! Agent harnesses used by Mixmod roles.
//!
//! A harness is the adapter that runs an external coding agent backend. Mixmod
//! strategy code decides whether an agent is acting as supervisor or worker;
//! harness code only executes the requested backend turn and reports what
//! happened.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;

use crate::{DelegationMode, SupervisorControlEvent};

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

/// Request passed from Mixmod strategy/run code to an agent harness.
#[derive(Debug)]
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
