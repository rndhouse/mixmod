use std::collections::BTreeSet;

use serde_json::Value;

use crate::SupervisorFeedback;

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
pub(crate) struct RevisionHandoff {
    pub(crate) expect_patch: Option<bool>,
    pub(crate) worker_turn_shape: Option<String>,
    pub(crate) turn_goal: Option<String>,
    pub(crate) exact_edits: Vec<String>,
    pub(crate) edit_plan: Vec<String>,
    pub(crate) deferred_checks: Vec<String>,
    pub(crate) defer_checks_until_patch_exists: Option<bool>,
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
            completion_gate: feedback.completion_gate.clone(),
            forbidden_actions: feedback.forbidden_actions.clone(),
        }
    }

    pub(crate) fn is_patch_request(&self) -> bool {
        if self.expect_patch == Some(false) {
            return false;
        }
        self.worker_turn_shape
            .as_deref()
            .is_some_and(|shape| shape.trim() == "patch_request")
    }

    pub(crate) fn is_bounded_feature_slice(&self) -> bool {
        if self.expect_patch == Some(false) {
            return false;
        }
        self.worker_turn_shape
            .as_deref()
            .is_some_and(|shape| shape.trim() == "bounded_feature_slice")
    }

    pub(crate) fn is_planning_probe(&self) -> bool {
        self.expect_patch == Some(false)
            && self
                .worker_turn_shape
                .as_deref()
                .is_some_and(|shape| shape.trim() == "planning_probe")
    }
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
