use crate::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The category of Mixmod-owned behavior that changed agent execution.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    /// Mixmod constructed the worker-facing task handoff.
    WorkerHandoff,
    /// Mixmod retried a patch-expected run that produced no patch.
    EmptyPatchFollowup,
    /// Mixmod retried a supervisor revision that produced no new delta.
    RevisionNoopFollowup,
    /// Mixmod asked the same worker session to review and clean its own diff.
    WorkerSelfReview,
}

impl InterventionKind {
    /// Returns the stable snake_case representation used in metrics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkerHandoff => "worker_handoff",
            Self::EmptyPatchFollowup => "empty_patch_followup",
            Self::RevisionNoopFollowup => "revision_noop_followup",
            Self::WorkerSelfReview => "worker_self_review",
        }
    }
}

/// The run phase where an intervention happened.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionPhase {
    /// Before the worker process is launched.
    PreWorker,
    /// After a worker process returns.
    PostWorker,
}

/// The actor whose inputs or execution were changed by Mixmod.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionTarget {
    /// The local worker process.
    Worker,
}

/// How an intervention handled agent session state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionSessionPolicy {
    /// The intervention started from a fresh worker session.
    FreshSession,
    /// The intervention reused an existing worker session.
    SameSession,
}

/// A single Mixmod-owned action that changed agent execution or review inputs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InterventionEvent {
    /// Event timestamp in RFC 3339 format.
    pub timestamp: String,
    /// Intervention category.
    pub kind: InterventionKind,
    /// Run phase where the intervention happened.
    pub phase: InterventionPhase,
    /// Actor whose inputs or execution were changed.
    pub target: InterventionTarget,
    /// Condition that caused the intervention.
    pub trigger: String,
    /// Session policy used by the intervention.
    pub session_policy: Option<InterventionSessionPolicy>,
    /// Whether Mixmod actually ran the intervention.
    pub performed: bool,
    /// Compact outcome label.
    pub outcome: String,
    /// Run-relative artifact paths produced or used by the intervention.
    pub artifacts: Vec<String>,
    /// Structured details that are useful for debugging but not stable API.
    pub details: Map<String, Value>,
}

impl InterventionEvent {
    /// Creates a new intervention event with a current timestamp.
    pub fn new(
        kind: InterventionKind,
        phase: InterventionPhase,
        target: InterventionTarget,
        trigger: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            kind,
            phase,
            target,
            trigger: trigger.into(),
            session_policy: None,
            performed: true,
            outcome: outcome.into(),
            artifacts: Vec::new(),
            details: Map::new(),
        }
    }

    /// Adds the session policy used by this intervention.
    pub fn with_session_policy(mut self, session_policy: InterventionSessionPolicy) -> Self {
        self.session_policy = Some(session_policy);
        self
    }

    /// Marks whether the intervention was fully performed.
    pub fn with_performed(mut self, performed: bool) -> Self {
        self.performed = performed;
        self
    }

    /// Adds run-relative artifact paths attached to this intervention.
    pub fn with_artifacts(mut self, artifacts: Vec<String>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// Adds structured debugging details.
    pub fn with_details(mut self, details: Map<String, Value>) -> Self {
        self.details = details;
        self
    }
}

/// In-memory collection that can be written as `interventions.jsonl`.
#[derive(Default)]
pub struct InterventionLog {
    events: Vec<InterventionEvent>,
}

impl InterventionLog {
    /// Creates an empty intervention log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a single intervention event.
    pub fn record(&mut self, event: InterventionEvent) {
        self.events.push(event);
    }

    /// Returns all recorded intervention events.
    pub fn events(&self) -> &[InterventionEvent] {
        &self.events
    }

    /// Returns a stable list of intervention kinds in first-seen order.
    pub fn kind_names(&self) -> Vec<String> {
        let mut kinds = Vec::new();
        for event in &self.events {
            let kind = event.kind.as_str().to_string();
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        kinds
    }

    /// Writes the log as newline-delimited JSON.
    pub fn write_jsonl(&self, path: &Path) -> Result<()> {
        let mut output = String::new();
        for event in &self.events {
            output.push_str(&serde_json::to_string(event)?);
            output.push('\n');
        }
        atomic_write(path, output.as_bytes())
    }
}
