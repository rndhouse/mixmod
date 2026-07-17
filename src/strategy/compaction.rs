use std::fs;
use std::path::PathBuf;

use crate::{
    SupervisorBriefTurn, SupervisorCompactionTurn, SupervisorContextTelemetry,
    SupervisorFeedbackTurn, env_bool, env_u64, get_str,
};

/// Requested supervisor compaction action chosen by Mixmod.
pub(crate) struct SupervisorCompactionRequest {
    /// Compact immediately before the next operation on the supervisor thread.
    pub(crate) timing: SupervisorCompactionTiming,
    /// Stable trigger label recorded in feedback artifacts.
    pub(crate) trigger: String,
    /// Supervisor recommendation that contributed to the decision.
    pub(crate) recommendation: serde_json::Value,
    /// Token/context telemetry at the decision point.
    pub(crate) telemetry: SupervisorContextTelemetry,
}

/// When to run a supervisor compaction request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupervisorCompactionTiming {
    /// Compact immediately after the current supervisor review.
    Now,
    /// Compact before the next supervisor review.
    BeforeNextReview,
}

/// Token and recommendation state for Codex supervisor compaction.
#[derive(Default)]
pub(crate) struct SupervisorCompactionState {
    compaction_count: u64,
    turns_since_last_compact: u64,
    input_since_last_compact: u64,
    cached_input_since_last_compact: u64,
    total_since_last_compact: u64,
    latest_input_tokens: u64,
    latest_cached_input_tokens: u64,
    latest_total_tokens: u64,
    deferred: Option<SupervisorCompactionRequest>,
}

impl SupervisorCompactionState {
    /// Build prompt-visible telemetry for the next supervisor decision.
    pub(crate) fn telemetry(&self, artifact_paths: &[PathBuf]) -> SupervisorContextTelemetry {
        let thresholds = SupervisorCompactionThresholds::from_env();
        SupervisorContextTelemetry {
            supervisor_turns_since_last_compact: self.turns_since_last_compact,
            supervisor_compaction_count: self.compaction_count,
            latest_supervisor_input_tokens: self.latest_input_tokens,
            latest_supervisor_cached_input_tokens: self.latest_cached_input_tokens,
            latest_supervisor_total_tokens: self.latest_total_tokens,
            supervisor_input_tokens_since_last_compact: self.input_since_last_compact,
            supervisor_cached_input_tokens_since_last_compact: self.cached_input_since_last_compact,
            supervisor_total_tokens_since_last_compact: self.total_since_last_compact,
            review_artifact_bytes: review_artifact_bytes(artifact_paths),
            compact_moderate_input_threshold: thresholds.moderate_input_tokens,
            compact_moderate_total_threshold: thresholds.moderate_total_tokens,
            compact_force_input_threshold: thresholds.force_input_tokens,
            compact_force_total_threshold: thresholds.force_total_tokens,
            compact_min_turns_threshold: thresholds.min_turns,
            compaction_enabled: thresholds.enabled,
        }
    }

    /// Record a normal supervisor brief turn.
    pub(crate) fn record_brief(&mut self, turn: &SupervisorBriefTurn) {
        self.record_supervisor_turn(
            turn.input_tokens,
            turn.cached_input_tokens,
            turn.total_tokens,
        );
    }

    /// Record a normal supervisor feedback turn.
    pub(crate) fn record_feedback(&mut self, turn: &SupervisorFeedbackTurn) {
        self.record_supervisor_turn(
            turn.input_tokens,
            turn.cached_input_tokens,
            turn.total_tokens,
        );
    }

    /// Record a compact turn and reset post-compaction pressure counters.
    pub(crate) fn record_compaction(&mut self, turn: &SupervisorCompactionTurn) {
        self.compaction_count += 1;
        self.turns_since_last_compact = 0;
        self.input_since_last_compact = 0;
        self.cached_input_since_last_compact = 0;
        self.total_since_last_compact = 0;
        self.latest_input_tokens = turn.input_tokens;
        self.latest_cached_input_tokens = turn.cached_input_tokens;
        self.latest_total_tokens = turn.total_tokens;
        self.deferred = None;
    }

    /// Return a deferred compaction request that should run before review.
    pub(crate) fn take_before_review_request(&mut self) -> Option<SupervisorCompactionRequest> {
        match self.deferred.take() {
            Some(request) if request.timing == SupervisorCompactionTiming::BeforeNextReview => {
                Some(request)
            }
            Some(request) => {
                self.deferred = Some(request);
                None
            }
            None => None,
        }
    }

    /// Decide whether a just-completed feedback turn should trigger compaction.
    pub(crate) fn request_after_feedback(
        &mut self,
        feedback: &SupervisorFeedbackTurn,
    ) -> Option<SupervisorCompactionRequest> {
        let thresholds = SupervisorCompactionThresholds::from_env();
        self.request_after_feedback_with_thresholds(feedback, &thresholds)
    }

    fn request_after_feedback_with_thresholds(
        &mut self,
        feedback: &SupervisorFeedbackTurn,
        thresholds: &SupervisorCompactionThresholds,
    ) -> Option<SupervisorCompactionRequest> {
        if !thresholds.enabled {
            return None;
        }

        let recommendation = context_recommendation_from_feedback(&feedback.feedback);
        let recommendation_action = get_str(&recommendation, "action").unwrap_or("continue");
        let severe = self.latest_input_tokens >= thresholds.force_input_tokens
            || self.total_since_last_compact >= thresholds.force_total_tokens;
        let moderate = self.latest_input_tokens >= thresholds.moderate_input_tokens
            || self.total_since_last_compact >= thresholds.moderate_total_tokens
            || (self.turns_since_last_compact >= thresholds.min_turns
                && self.latest_input_tokens >= thresholds.min_turn_input_tokens);

        let telemetry = self.telemetry(&[]);
        if severe {
            return Some(SupervisorCompactionRequest {
                timing: SupervisorCompactionTiming::Now,
                trigger: "forced_supervisor_context_pressure".to_string(),
                recommendation,
                telemetry,
            });
        }
        if !moderate {
            return None;
        }
        match recommendation_action {
            "compact_now" => Some(SupervisorCompactionRequest {
                timing: SupervisorCompactionTiming::Now,
                trigger: "supervisor_recommended_compact_now".to_string(),
                recommendation,
                telemetry,
            }),
            "compact_after_next_worker" => {
                let request = SupervisorCompactionRequest {
                    timing: SupervisorCompactionTiming::BeforeNextReview,
                    trigger: "supervisor_recommended_compact_after_next_worker".to_string(),
                    recommendation,
                    telemetry,
                };
                self.deferred = Some(request.clone_for_storage());
                None
            }
            _ => None,
        }
    }

    fn record_supervisor_turn(
        &mut self,
        input_tokens: u64,
        cached_input_tokens: u64,
        total_tokens: u64,
    ) {
        self.turns_since_last_compact += 1;
        self.input_since_last_compact += input_tokens;
        self.cached_input_since_last_compact += cached_input_tokens;
        self.total_since_last_compact += total_tokens;
        self.latest_input_tokens = input_tokens;
        self.latest_cached_input_tokens = cached_input_tokens;
        self.latest_total_tokens = total_tokens;
    }
}

impl SupervisorCompactionRequest {
    fn clone_for_storage(&self) -> Self {
        Self {
            timing: self.timing,
            trigger: self.trigger.clone(),
            recommendation: self.recommendation.clone(),
            telemetry: self.telemetry.clone(),
        }
    }
}

struct SupervisorCompactionThresholds {
    enabled: bool,
    moderate_input_tokens: u64,
    moderate_total_tokens: u64,
    force_input_tokens: u64,
    force_total_tokens: u64,
    min_turns: u64,
    min_turn_input_tokens: u64,
}

impl SupervisorCompactionThresholds {
    fn from_env() -> Self {
        Self {
            enabled: env_bool("MIXMOD_SUPERVISOR_COMPACT").unwrap_or(true),
            moderate_input_tokens: env_u64("MIXMOD_SUPERVISOR_COMPACT_INPUT_TOKENS")
                .unwrap_or(160_000),
            moderate_total_tokens: env_u64("MIXMOD_SUPERVISOR_COMPACT_TOTAL_TOKENS")
                .unwrap_or(600_000),
            force_input_tokens: env_u64("MIXMOD_SUPERVISOR_COMPACT_FORCE_INPUT_TOKENS")
                .unwrap_or(240_000),
            force_total_tokens: env_u64("MIXMOD_SUPERVISOR_COMPACT_FORCE_TOTAL_TOKENS")
                .unwrap_or(1_000_000),
            min_turns: env_u64("MIXMOD_SUPERVISOR_COMPACT_MIN_TURNS").unwrap_or(4),
            min_turn_input_tokens: env_u64("MIXMOD_SUPERVISOR_COMPACT_MIN_TURN_INPUT_TOKENS")
                .unwrap_or(100_000),
        }
    }
}

fn context_recommendation_from_feedback(feedback_record: &serde_json::Value) -> serde_json::Value {
    let feedback = feedback_record.get("feedback").unwrap_or(feedback_record);
    let raw = feedback.get("context_recommendation");
    let (action, reason) = match raw {
        Some(serde_json::Value::Object(map)) => (
            normalize_context_recommendation_action(
                map.get("action").and_then(serde_json::Value::as_str),
            ),
            map.get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
        ),
        Some(serde_json::Value::String(value)) => (
            normalize_context_recommendation_action(Some(value)),
            String::new(),
        ),
        _ => ("continue".to_string(), String::new()),
    };
    serde_json::json!({
        "action": action,
        "reason": reason,
    })
}

fn normalize_context_recommendation_action(value: Option<&str>) -> String {
    match value
        .unwrap_or("continue")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "compact_now" | "compact" | "now" => "compact_now".to_string(),
        "compact_after_next_worker" | "after_next_worker" | "before_next_review" => {
            "compact_after_next_worker".to_string()
        }
        _ => "continue".to_string(),
    }
}

fn review_artifact_bytes(artifact_paths: &[PathBuf]) -> u64 {
    artifact_paths
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RevisionHandoff;
    use serde_json::json;

    fn thresholds() -> SupervisorCompactionThresholds {
        SupervisorCompactionThresholds {
            enabled: true,
            moderate_input_tokens: 100,
            moderate_total_tokens: 300,
            force_input_tokens: 500,
            force_total_tokens: 1_000,
            min_turns: 3,
            min_turn_input_tokens: 80,
        }
    }

    fn feedback(context_recommendation: serde_json::Value) -> SupervisorFeedbackTurn {
        SupervisorFeedbackTurn {
            feedback: json!({
                "feedback": {
                    "action": "revise",
                    "context_recommendation": context_recommendation
                }
            }),
            verdict: "revise".to_string(),
            worker_mode: "continue".to_string(),
            patch_decision: "accept_current".to_string(),
            hint: String::new(),
            revision_handoff: RevisionHandoff::default(),
            focus_files: Vec::new(),
            required_checks: Vec::new(),
            supervisor_direct_edit_reason: None,
            direct_plan: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            total_tokens: 0,
            cached_input_tokens: 0,
            input_bytes: 0,
            output_bytes: 0,
            thread_id: String::new(),
            turn_id: String::new(),
            token_usage_comparable: true,
        }
    }

    #[test]
    fn compaction_policy_forces_on_hard_context_pressure() {
        let mut state = SupervisorCompactionState::default();
        state.record_supervisor_turn(600, 500, 700);

        let request = state
            .request_after_feedback_with_thresholds(&feedback(json!("continue")), &thresholds())
            .unwrap();

        assert_eq!(request.timing, SupervisorCompactionTiming::Now);
        assert_eq!(request.trigger, "forced_supervisor_context_pressure");
        assert_eq!(get_str(&request.recommendation, "action"), Some("continue"));
    }

    #[test]
    fn compaction_policy_defers_after_next_worker_when_recommended() {
        let mut state = SupervisorCompactionState::default();
        state.record_supervisor_turn(120, 80, 150);

        let request = state.request_after_feedback_with_thresholds(
            &feedback(json!({
                "action": "compact_after_next_worker",
                "reason": "clean next worker slice"
            })),
            &thresholds(),
        );

        assert!(request.is_none());
        let deferred = state.take_before_review_request().unwrap();
        assert_eq!(
            deferred.timing,
            SupervisorCompactionTiming::BeforeNextReview
        );
        assert_eq!(
            deferred.trigger,
            "supervisor_recommended_compact_after_next_worker"
        );
    }
}
