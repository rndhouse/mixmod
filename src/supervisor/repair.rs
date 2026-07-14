use serde_json::Value;

use crate::*;

use super::normalize::normalize_supervisor_verdict;
use super::types::{RevisionHandoff, SupervisorVerdict, WorkerTurnShape};

pub(super) fn worker_brief_needs_patch_request_repair(
    brief: &Value,
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    if worker_guidance_prefers_bounded_feature_slice(worker_guidance) {
        return false;
    }
    if !worker_guidance_prefers_patch_request(worker_guidance) {
        return false;
    }
    let typed = WorkerBrief::from_value(brief);
    let handoff = typed.handoff.as_deref().unwrap_or("guided");
    if handoff == "blocked" {
        return false;
    }
    let expect_patch = typed.expect_patch.unwrap_or(handoff != "as_given");
    if !expect_patch
        && WorkerTurnShape::from_raw(typed.worker_turn_shape.as_deref())
            == Some(WorkerTurnShape::PlanningProbe)
    {
        return false;
    }
    let patch_request = WorkerTurnShape::from_raw(typed.worker_turn_shape.as_deref())
        == Some(WorkerTurnShape::PatchRequest);
    if !expect_patch {
        return false;
    }
    if !patch_request {
        return true;
    }
    if patch_request_instruction_count(&[
        &typed.exact_edits,
        &typed.edit_plan,
        &typed.implementation_steps,
    ]) > 3
    {
        return true;
    }
    !worker_brief_has_patch_request_goal(&typed)
}

pub(super) fn supervisor_feedback_needs_patch_request_repair(
    feedback: &Value,
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    let raw_verdict = get_str(feedback, "verdict")
        .or_else(|| get_str(feedback, "action"))
        .unwrap_or("revise");
    if normalize_supervisor_verdict(raw_verdict) != SupervisorVerdict::Revise {
        return false;
    }
    if worker_guidance_prefers_bounded_feature_slice(worker_guidance) {
        return false;
    }
    if !worker_guidance_prefers_patch_request(worker_guidance) {
        return false;
    }
    let typed = SupervisorFeedback::from_value(feedback);
    if typed.expect_patch == Some(false)
        && WorkerTurnShape::from_raw(typed.worker_turn_shape.as_deref())
            == Some(WorkerTurnShape::PlanningProbe)
    {
        return false;
    }
    let handoff = RevisionHandoff::from_feedback(&typed);
    if !handoff.is_patch_request() {
        return true;
    }
    if patch_request_instruction_count(&[&typed.exact_edits, &typed.edit_plan]) > 3 {
        return true;
    }
    !supervisor_feedback_has_patch_request_goal(&typed)
}

fn worker_guidance_prefers_patch_request(worker_guidance: &WorkerSupervisorGuidance) -> bool {
    worker_guidance.guidance.iter().any(|item| {
        item.contains("worker_turn_shape=patch_request")
            || item.contains("one immediate source edit")
    })
}

fn worker_guidance_prefers_bounded_feature_slice(
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    worker_guidance
        .guidance
        .iter()
        .any(|item| item.contains("worker_turn_shape=bounded_feature_slice"))
}

fn patch_request_instruction_count(lists: &[&Vec<String>]) -> usize {
    lists
        .iter()
        .flat_map(|items| items.iter())
        .filter(|item| !item.trim().is_empty())
        .count()
}

fn non_empty_optional(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|item| !item.trim().is_empty())
}

fn worker_brief_has_patch_request_goal(brief: &WorkerBrief) -> bool {
    non_empty_optional(&brief.turn_goal)
        || non_empty_optional(&brief.objective)
        || non_empty_optional(&brief.message_to_worker)
        || non_empty_optional(&brief.message)
        || non_empty_optional(&brief.supplement)
        || patch_request_instruction_count(&[
            &brief.exact_edits,
            &brief.edit_plan,
            &brief.implementation_steps,
        ]) > 0
}

fn supervisor_feedback_has_patch_request_goal(feedback: &SupervisorFeedback) -> bool {
    non_empty_optional(&feedback.turn_goal)
        || non_empty_optional(&feedback.message_to_worker)
        || non_empty_optional(&feedback.hint)
        || patch_request_instruction_count(&[&feedback.exact_edits, &feedback.edit_plan]) > 0
}

pub(super) fn revision_repair_preserves_focus(previous: &Value, repaired: &Value) -> bool {
    let previous_feedback = SupervisorFeedback::from_value(previous);
    let previous_focus = previous_feedback
        .focus_files
        .iter()
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    if previous_focus.len() != 1 {
        return true;
    }

    let target = previous_focus[0].as_str();
    let repaired_feedback = SupervisorFeedback::from_value(repaired);
    repaired_feedback
        .focus_files
        .iter()
        .any(|path| path == target)
        || repaired_feedback
            .exact_edits
            .iter()
            .any(|edit| edit.contains(target))
}

pub(super) fn repaired_brief_is_accepted(
    repaired_brief: Option<&Value>,
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    repaired_brief
        .is_some_and(|brief| !worker_brief_needs_patch_request_repair(brief, worker_guidance))
}

pub(super) fn worker_brief_repair_rejection_reason(
    repaired_brief: Option<&Value>,
    worker_guidance: &WorkerSupervisorGuidance,
) -> String {
    match repaired_brief {
        None => "The repaired handoff was not parseable JSON.".to_string(),
        Some(brief) if worker_brief_needs_patch_request_repair(brief, worker_guidance) => {
            "The repaired handoff still does not satisfy the expected patch_request shape: provide a bounded patch goal and compact optional edit details.".to_string()
        }
        Some(_) => "The repaired handoff was rejected by structural repair checks.".to_string(),
    }
}

pub(super) fn repaired_feedback_is_accepted(
    previous_feedback: &Value,
    repaired_feedback: Option<&Value>,
    worker_guidance: &WorkerSupervisorGuidance,
) -> bool {
    repaired_feedback.is_some_and(|feedback| {
        !supervisor_feedback_needs_patch_request_repair(feedback, worker_guidance)
            && revision_repair_preserves_focus(previous_feedback, feedback)
    })
}

pub(super) fn supervisor_feedback_repair_rejection_reason(
    previous_feedback: &Value,
    repaired_feedback: Option<&Value>,
    worker_guidance: &WorkerSupervisorGuidance,
) -> String {
    match repaired_feedback {
        None => "The repaired revision decision was not parseable JSON.".to_string(),
        Some(feedback)
            if supervisor_feedback_needs_patch_request_repair(feedback, worker_guidance) =>
        {
            "The repaired revision still does not satisfy the expected patch_request shape: provide a bounded patch goal and compact optional edit details.".to_string()
        }
        Some(feedback) if !revision_repair_preserves_focus(previous_feedback, feedback) => {
            "The repaired revision changed away from the previous single focus file; preserve that target unless the artifacts prove it is wrong.".to_string()
        }
        Some(_) => "The repaired revision was rejected by structural repair checks.".to_string(),
    }
}
