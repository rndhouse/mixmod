use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::SupervisorFeedback;

/// Normalized supervisor decision for the worker loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupervisorVerdict {
    /// The accumulated patch is accepted.
    Approve,
    /// The worker should make a focused repository edit.
    WorkerEdit,
    /// The worker should inspect, verify, or plan without editing.
    WorkerInspect,
    /// The supervisor should execute a bounded surgical direct edit.
    SupervisorDirectEdit,
    /// The loop should stop without approval.
    Stop,
}

impl SupervisorVerdict {
    /// Return the stable artifact string for this verdict.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::WorkerEdit => "worker_edit",
            Self::WorkerInspect => "worker_inspect",
            Self::SupervisorDirectEdit => "supervisor_direct_edit",
            Self::Stop => "stop",
        }
    }

    /// Parse a loose model-produced verdict into the protocol enum.
    pub(crate) fn from_raw(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "approve" | "approved" => Self::Approve,
            "stop" | "stopped" | "halt" | "done" | "needs_user" | "needs-user" => Self::Stop,
            "supervisor_direct_edit"
            | "supervisor-direct-edit"
            | "supervisor_direct"
            | "supervisor-direct"
            | "take_over"
            | "take-over"
            | "takeover"
            | "direct_finish"
            | "direct-finish" => Self::SupervisorDirectEdit,
            "worker_inspect" | "worker-inspect" | "worker_review" | "worker-review" | "inspect"
            | "inspection" | "planning_probe" | "planning-probe" => Self::WorkerInspect,
            "worker_edit" | "worker-edit" | "edit" | "revise" | "revision" | "needs_revision"
            | "needs-review" | "needs_review" | "reject" | "rejected" => Self::WorkerEdit,
            _ => Self::WorkerEdit,
        }
    }

    /// Return whether the ordinary worker loop should not start immediately.
    pub(crate) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Approve | Self::Stop | Self::SupervisorDirectEdit
        )
    }
}

/// Normalized worker session mode requested by the supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerMode {
    /// Continue the current worker backend session.
    Continue,
    /// Start a fresh worker backend session on the current worktree.
    ContextFocus,
}

impl WorkerMode {
    /// Return the stable artifact string for this worker mode.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::ContextFocus => "context_focus",
        }
    }

    /// Parse a loose model-produced worker mode into the protocol enum.
    pub(crate) fn from_raw(value: Option<&str>) -> Self {
        match value
            .unwrap_or("continue")
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .as_str()
        {
            "context_focus" | "focused" | "focus" | "fresh" | "reset" => Self::ContextFocus,
            _ => Self::Continue,
        }
    }
}

/// Normalized patch checkpoint decision for the next worker turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PatchDecision {
    /// Keep the current active worktree patch.
    AcceptCurrent,
    /// Commit the current patch as an internal baseline before revising.
    AcceptCurrentBaseline,
    /// Continue revising the current active patch.
    ReviseCurrent,
    /// Restore the previous candidate patch before revising.
    RevisePrevious,
}

impl PatchDecision {
    /// Return the stable artifact string for this patch decision.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AcceptCurrent => "accept_current",
            Self::AcceptCurrentBaseline => "accept_current_baseline",
            Self::ReviseCurrent => "revise_current",
            Self::RevisePrevious => "revise_previous",
        }
    }

    /// Parse a loose model-produced patch decision into the protocol enum.
    pub(crate) fn from_raw(value: Option<&str>) -> Self {
        match value
            .unwrap_or("accept_current")
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .as_str()
        {
            "accept_current_baseline"
            | "accept_current_as_baseline"
            | "checkpoint_current"
            | "baseline_current"
            | "commit_current_baseline" => Self::AcceptCurrentBaseline,
            "revise_previous" | "previous" | "keep_previous" | "restore_previous"
            | "recover_previous" => Self::RevisePrevious,
            "revise_current" | "current_revision" | "continue_current" => Self::ReviseCurrent,
            _ => Self::AcceptCurrent,
        }
    }
}

/// Normalized shape of the next worker turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerTurnShape {
    /// Ask the worker to inspect narrowly and propose the next patch request.
    PlanningProbe,
    /// Ask for one focused source edit.
    PatchRequest,
    /// Ask for a coherent bounded feature chunk.
    BoundedFeatureSlice,
    /// Use the default worker instruction shape.
    Default,
}

impl WorkerTurnShape {
    /// Parse a model-produced worker-turn shape when one was provided.
    pub(crate) fn from_raw(value: Option<&str>) -> Option<Self> {
        match value?.trim() {
            "planning_probe" => Some(Self::PlanningProbe),
            "patch_request" => Some(Self::PatchRequest),
            "bounded_feature_slice" => Some(Self::BoundedFeatureSlice),
            "default" => Some(Self::Default),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct SupervisorFeedbackTurn {
    pub(crate) feedback: Value,
    pub(crate) verdict: String,
    pub(crate) worker_mode: String,
    pub(crate) patch_decision: String,
    pub(crate) hint: String,
    pub(crate) revision_handoff: RevisionHandoff,
    pub(crate) focus_files: Vec<String>,
    pub(crate) required_checks: Vec<String>,
    pub(crate) supervisor_direct_edit_reason: Option<String>,
    pub(crate) direct_plan: Vec<String>,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) token_usage_comparable: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SupervisorContextTelemetry {
    pub(crate) supervisor_turns_since_last_compact: u64,
    pub(crate) supervisor_compaction_count: u64,
    pub(crate) latest_supervisor_input_tokens: u64,
    pub(crate) latest_supervisor_cached_input_tokens: u64,
    pub(crate) latest_supervisor_total_tokens: u64,
    pub(crate) supervisor_input_tokens_since_last_compact: u64,
    pub(crate) supervisor_cached_input_tokens_since_last_compact: u64,
    pub(crate) supervisor_total_tokens_since_last_compact: u64,
    pub(crate) review_artifact_bytes: u64,
    pub(crate) compact_moderate_input_threshold: u64,
    pub(crate) compact_moderate_total_threshold: u64,
    pub(crate) compact_force_input_threshold: u64,
    pub(crate) compact_force_total_threshold: u64,
    pub(crate) compact_min_turns_threshold: u64,
    pub(crate) compaction_enabled: bool,
}

impl SupervisorContextTelemetry {
    pub(crate) fn to_prompt_json(&self) -> Value {
        json!({
            "supervisor_turns_since_last_compact": self.supervisor_turns_since_last_compact,
            "supervisor_compaction_count": self.supervisor_compaction_count,
            "latest_supervisor_input_tokens": self.latest_supervisor_input_tokens,
            "latest_supervisor_cached_input_tokens": self.latest_supervisor_cached_input_tokens,
            "latest_supervisor_total_tokens": self.latest_supervisor_total_tokens,
            "supervisor_input_tokens_since_last_compact": self.supervisor_input_tokens_since_last_compact,
            "supervisor_cached_input_tokens_since_last_compact": self.supervisor_cached_input_tokens_since_last_compact,
            "supervisor_total_tokens_since_last_compact": self.supervisor_total_tokens_since_last_compact,
            "review_artifact_bytes": self.review_artifact_bytes,
            "compact_moderate_input_threshold": self.compact_moderate_input_threshold,
            "compact_moderate_total_threshold": self.compact_moderate_total_threshold,
            "compact_force_input_threshold": self.compact_force_input_threshold,
            "compact_force_total_threshold": self.compact_force_total_threshold,
            "compact_min_turns_threshold": self.compact_min_turns_threshold,
            "compaction_enabled": self.compaction_enabled,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RevisionHandoff {
    pub(crate) expect_patch: Option<bool>,
    pub(crate) worker_turn_shape: Option<String>,
    pub(crate) turn_goal: Option<String>,
    pub(crate) exact_edits: Vec<String>,
    pub(crate) edit_plan: Vec<String>,
    pub(crate) deferred_checks: Vec<String>,
    pub(crate) defer_checks_until_patch_exists: Option<bool>,
    pub(crate) stop_condition: Option<String>,
    pub(crate) completion_gate: Option<String>,
    pub(crate) forbidden_actions: Vec<String>,
}

impl RevisionHandoff {
    pub(crate) fn from_feedback(feedback: &SupervisorFeedback) -> Self {
        Self {
            expect_patch: feedback.expect_patch,
            worker_turn_shape: feedback.worker_turn_shape.clone(),
            turn_goal: feedback.turn_goal.clone(),
            exact_edits: feedback.exact_edits.clone(),
            edit_plan: feedback.edit_plan.clone(),
            deferred_checks: feedback.deferred_checks.clone(),
            defer_checks_until_patch_exists: feedback.defer_checks_until_patch_exists,
            stop_condition: feedback.stop_condition.clone(),
            completion_gate: feedback.completion_gate.clone(),
            forbidden_actions: feedback.forbidden_actions.clone(),
        }
    }

    pub(crate) fn is_patch_request(&self) -> bool {
        if self.expect_patch == Some(false) {
            return false;
        }
        self.worker_turn_shape() == Some(WorkerTurnShape::PatchRequest)
    }

    pub(crate) fn is_bounded_feature_slice(&self) -> bool {
        if self.expect_patch == Some(false) {
            return false;
        }
        self.worker_turn_shape() == Some(WorkerTurnShape::BoundedFeatureSlice)
    }

    pub(crate) fn is_planning_probe(&self) -> bool {
        self.expect_patch == Some(false)
            && self.worker_turn_shape() == Some(WorkerTurnShape::PlanningProbe)
    }

    /// Return why this handoff must start a fresh worker session, if any.
    pub(crate) fn fresh_session_reason(&self, message_to_worker: &str) -> Option<&'static str> {
        if self.is_planning_probe() {
            return Some("planning_probe");
        }
        if self.expect_patch == Some(false) {
            return Some("expect_patch_false");
        }
        if self.is_verification_only(message_to_worker) {
            return Some("verification_only");
        }
        if self.contains_session_boundary_forbidden_action(message_to_worker) {
            return Some("forbidden_action_session_boundary");
        }
        None
    }

    /// Return whether this handoff must start a fresh worker session.
    pub(crate) fn requires_fresh_worker_session(&self, message_to_worker: &str) -> bool {
        self.fresh_session_reason(message_to_worker).is_some()
    }

    /// Return whether no repository delta should be expected for this handoff.
    pub(crate) fn suppresses_revision_delta(&self, message_to_worker: &str) -> bool {
        self.is_planning_probe()
            || self.expect_patch == Some(false)
            || self.forbids_repository_edits(message_to_worker)
            || self.is_verification_only(message_to_worker)
    }

    /// Return the normalized worker turn shape, when one was supplied.
    pub(crate) fn worker_turn_shape(&self) -> Option<WorkerTurnShape> {
        WorkerTurnShape::from_raw(self.worker_turn_shape.as_deref())
    }

    fn contains_session_boundary_forbidden_action(&self, message_to_worker: &str) -> bool {
        self.session_policy_texts(message_to_worker)
            .any(session_boundary_forbidden_action)
    }

    fn forbids_repository_edits(&self, message_to_worker: &str) -> bool {
        self.session_policy_texts(message_to_worker)
            .any(|text| normalized_policy_text(text).contains("do not edit"))
    }

    fn is_verification_only(&self, message_to_worker: &str) -> bool {
        self.session_policy_texts(message_to_worker)
            .any(verification_only_text)
    }

    fn session_policy_texts<'a>(
        &'a self,
        message_to_worker: &'a str,
    ) -> impl Iterator<Item = &'a str> {
        std::iter::once(message_to_worker)
            .chain(self.turn_goal.as_deref())
            .chain(self.stop_condition.as_deref())
            .chain(self.completion_gate.as_deref())
            .chain(self.exact_edits.iter().map(String::as_str))
            .chain(self.edit_plan.iter().map(String::as_str))
            .chain(self.forbidden_actions.iter().map(String::as_str))
    }
}

impl SupervisorFeedbackTurn {
    /// Return the normalized verdict for this turn.
    pub(crate) fn verdict_kind(&self) -> SupervisorVerdict {
        SupervisorVerdict::from_raw(&self.verdict)
    }

    /// Return the normalized worker mode for this turn.
    pub(crate) fn worker_mode_kind(&self) -> WorkerMode {
        WorkerMode::from_raw(Some(&self.worker_mode))
    }

    /// Return the normalized patch decision for this turn.
    pub(crate) fn patch_decision_kind(&self) -> PatchDecision {
        PatchDecision::from_raw(Some(&self.patch_decision))
    }

    /// Return why the next worker turn must start a fresh session, if any.
    pub(crate) fn fresh_worker_session_reason(&self) -> Option<&'static str> {
        if self.is_supervisor_control_turn() {
            return Some("supervisor_control");
        }
        self.revision_handoff.fresh_session_reason(&self.hint)
    }

    /// Return whether the next worker turn must start a fresh session.
    pub(crate) fn requires_fresh_worker_session(&self) -> bool {
        self.fresh_worker_session_reason().is_some()
    }

    fn is_supervisor_control_turn(&self) -> bool {
        value_str(&self.feedback, "label") == Some("supervisor-control")
            || value_str(&self.feedback, "supervisor_control_action").is_some()
            || self.feedback.get("source_run").is_some()
    }
}

fn value_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn session_boundary_forbidden_action(text: &str) -> bool {
    let normalized = normalized_policy_text(text);
    [
        "do not edit",
        "do not run",
        "do not commit",
        "do not switch branches",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn verification_only_text(text: &str) -> bool {
    let normalized = normalized_policy_text(text);
    [
        "verification only",
        "verify only",
        "only verify",
        "verification step only",
        "verification turn only",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn normalized_policy_text(text: &str) -> String {
    text.trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug)]
pub(crate) struct SupervisorBriefTurn {
    pub(crate) record: Value,
    pub(crate) brief: Value,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) token_usage_comparable: bool,
}

#[derive(Debug)]
pub(crate) struct SupervisorCompactionTurn {
    pub(crate) record: Value,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) token_usage_comparable: bool,
}

#[derive(Clone)]
pub(crate) struct SupervisorUsageSample {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) reasoning_tokens: u64,
    pub(super) total_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) input_bytes: u64,
    pub(super) output_bytes: u64,
    pub(super) thread_id: String,
    pub(super) turn_id: String,
    pub(super) token_usage_comparable: bool,
}

impl SupervisorFeedbackTurn {
    pub(crate) fn usage_sample(&self) -> SupervisorUsageSample {
        SupervisorUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
            token_usage_comparable: self.token_usage_comparable,
        }
    }
}

impl SupervisorBriefTurn {
    pub(crate) fn usage_sample(&self) -> SupervisorUsageSample {
        SupervisorUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
            token_usage_comparable: self.token_usage_comparable,
        }
    }
}

impl SupervisorCompactionTurn {
    pub(crate) fn usage_sample(&self) -> SupervisorUsageSample {
        SupervisorUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
            token_usage_comparable: self.token_usage_comparable,
        }
    }
}

#[derive(Default)]
pub(crate) struct SupervisorUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) turn_count: u64,
    pub(crate) thread_ids: Vec<String>,
    pub(crate) turn_ids: Vec<String>,
    pub(crate) token_usage_comparable: bool,
}

impl SupervisorUsage {
    pub(crate) fn thread_count(&self) -> u64 {
        self.thread_ids
            .iter()
            .filter(|id| !id.is_empty())
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            .len() as u64
    }

    pub(crate) fn thread_reuse_count(&self) -> u64 {
        let observed_thread_ids = self.thread_ids.iter().filter(|id| !id.is_empty()).count() as u64;
        let thread_count = self.thread_count();
        observed_thread_ids.saturating_sub(thread_count)
    }

    pub(crate) fn session_reused(&self) -> bool {
        self.thread_reuse_count() > 0
    }
}

pub(crate) fn aggregate_supervisor_usage(turns: &[SupervisorUsageSample]) -> SupervisorUsage {
    let mut usage = SupervisorUsage {
        token_usage_comparable: !turns.is_empty(),
        ..SupervisorUsage::default()
    };
    for turn in turns {
        usage.input_tokens += turn.input_tokens;
        usage.output_tokens += turn.output_tokens;
        usage.reasoning_tokens += turn.reasoning_tokens;
        usage.total_tokens += turn.total_tokens;
        usage.cached_input_tokens += turn.cached_input_tokens;
        usage.input_bytes += turn.input_bytes;
        usage.output_bytes += turn.output_bytes;
        usage.turn_count += 1;
        usage.thread_ids.push(turn.thread_id.clone());
        usage.turn_ids.push(turn.turn_id.clone());
        usage.token_usage_comparable &= turn.token_usage_comparable;
    }
    usage
}
