use std::path::Path;

use serde_json::json;

use crate::{
    LiveSupervisionConfig, LiveWorkerSnapshot, WorkerSupervisorGuidance, get_str, get_string_array,
};

use super::live::{
    live_supervision_snapshot_should_check, normalize_live_control_focus_files,
    sanitize_live_control_message, should_force_final_no_delta_live_stop,
    snapshot_for_live_supervisor_prompt,
};
use super::normalize::parse_feedback_json;
use super::repair::{
    revision_repair_preserves_focus, supervisor_feedback_needs_revision_slice_repair,
    supervisor_feedback_repair_rejection_reason, worker_brief_needs_small_slice_repair,
    worker_brief_repair_rejection_reason,
};
use super::turns::{
    approval_consistency_rejection, approval_consistency_repair_is_accepted,
    verification_revision_for_inconsistent_approval,
};
use super::*;

fn small_slice_guidance() -> WorkerSupervisorGuidance {
    WorkerSupervisorGuidance {
        model: "qwen".to_string(),
        guidance: vec![
            "For broad expected-patch tasks, prefer worker_turn_shape=small_patch_slice."
                .to_string(),
        ],
    }
}

#[test]
fn broad_qwen_worker_brief_needs_small_slice_repair() {
    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "worker_turn_shape": "default",
        "message_to_worker": "Implement the feature."
    });

    assert!(worker_brief_needs_small_slice_repair(
        &brief,
        &small_slice_guidance()
    ));
}

#[test]
fn complex_unknown_worker_brief_does_not_need_small_slice_repair() {
    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "message_to_worker": "Implement generated serialization and deserialization support.",
        "edit_plan": [
            "Update code generation for alias-sensitive packing.",
            "Add validation for nested optional metadata.",
            "Handle unpack defaults."
        ]
    });

    assert!(!worker_brief_needs_small_slice_repair(
        &brief,
        &WorkerSupervisorGuidance::default()
    ));
}

#[test]
fn broad_small_slice_worker_brief_still_needs_repair() {
    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "worker_turn_shape": "small_patch_slice",
        "exact_edits": [
            "In builder.py add flatten option helpers for flatten_prefix and flatten_rename validation.",
            "In builder.py add pack support.",
            "In builder.py add unpack support.",
            "Add tests for prefix and rename."
        ]
    });

    assert!(worker_brief_needs_small_slice_repair(
        &brief,
        &small_slice_guidance()
    ));
}

#[test]
fn small_slice_worker_brief_does_not_need_repair() {
    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "worker_turn_shape": "small_patch_slice",
        "exact_edits": ["Edit helper.py."]
    });

    assert!(!worker_brief_needs_small_slice_repair(
        &brief,
        &small_slice_guidance()
    ));
}

#[test]
fn live_supervision_waits_when_worker_has_fresh_delta() {
    let snapshot = LiveWorkerSnapshot {
        elapsed_ms: 300_000,
        new_delta_bytes: 120,
        repeated_read_count: 12,
        last_output_age_ms: 10_000,
        stdout_bytes: 2000,
        ..LiveWorkerSnapshot::default()
    };

    assert!(!live_supervision_snapshot_should_check(
        &snapshot,
        &LiveSupervisionConfig::default()
    ));
}

#[test]
fn live_supervision_checks_stale_worker_with_new_delta() {
    let snapshot = LiveWorkerSnapshot {
        elapsed_ms: 300_000,
        new_delta_bytes: 120,
        last_output_age_ms: 120_000,
        stdout_bytes: 2000,
        ..LiveWorkerSnapshot::default()
    };

    assert!(live_supervision_snapshot_should_check(
        &snapshot,
        &LiveSupervisionConfig::default()
    ));
}

#[test]
fn live_supervision_checks_repeated_no_delta_reads() {
    let snapshot = LiveWorkerSnapshot {
        elapsed_ms: 300_000,
        new_delta_bytes: 0,
        repeated_read_count: 4,
        repeated_read_signature: Some("read: src/lib.rs".to_string()),
        ..LiveWorkerSnapshot::default()
    };

    assert!(live_supervision_snapshot_should_check(
        &snapshot,
        &LiveSupervisionConfig::default()
    ));
}

#[test]
fn live_supervision_checks_no_delta_context_overflow() {
    let snapshot = LiveWorkerSnapshot {
        elapsed_ms: 130_000,
        new_delta_bytes: 0,
        context_overflow_count: 1,
        ..LiveWorkerSnapshot::default()
    };

    assert!(live_supervision_snapshot_should_check(
        &snapshot,
        &LiveSupervisionConfig::default()
    ));
}

#[test]
fn final_live_no_delta_check_forces_stop_after_prior_segment() {
    let snapshot = LiveWorkerSnapshot {
        live_control_check_index: 3,
        live_control_check_limit: 3,
        opencode_segment: 2,
        new_delta_bytes: 0,
        ..LiveWorkerSnapshot::default()
    };

    assert!(should_force_final_no_delta_live_stop(
        &snapshot,
        "interrupt_continue"
    ));
}

#[test]
fn final_live_no_delta_stop_guard_ignores_first_segment() {
    let snapshot = LiveWorkerSnapshot {
        live_control_check_index: 3,
        live_control_check_limit: 3,
        opencode_segment: 1,
        new_delta_bytes: 0,
        ..LiveWorkerSnapshot::default()
    };

    assert!(!should_force_final_no_delta_live_stop(
        &snapshot,
        "interrupt_continue"
    ));
}

#[test]
fn live_supervisor_prompt_snapshot_redacts_artifact_paths() {
    let snapshot = LiveWorkerSnapshot {
        out_dir: "/tmp/mixmod-state/projects/app/runs/run-1/worker-runs/proposal".to_string(),
        task_path: "/tmp/mixmod-state/projects/app/runs/run-1/worker-task.json".to_string(),
        worker_instruction_excerpt:
            "Original task instructions: Add a flatten option to field_options.".to_string(),
        live_control_check_index: 3,
        live_control_check_limit: 3,
        elapsed_ms: 130_000,
        context_overflow_count: 1,
        ..LiveWorkerSnapshot::default()
    };
    let prompt = supervisor_live_control_prompt(
        Path::new("/app"),
        &snapshot_for_live_supervisor_prompt(&snapshot),
        &WorkerSupervisorGuidance::default(),
    )
    .unwrap();

    assert!(prompt.contains("It cannot read Mixmod task, state, log, or artifact paths"));
    assert!(prompt.contains("Original task instructions: Add a flatten option"));
    assert!(prompt.contains("Do not invent a different cleanup, bug, or objective"));
    assert!(prompt.contains("live_control_check_index equals live_control_check_limit"));
    assert!(!prompt.contains("/tmp/mixmod-state/projects/app/runs/run-1"));
    assert!(prompt.contains("[redacted: Mixmod artifact directory]"));
    assert!(prompt.contains("[redacted: Mixmod worker task artifact]"));
}

#[test]
fn live_control_focus_files_stay_repo_relative() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();
    let files = normalize_live_control_focus_files(
        root,
        vec![
            root.join("src/lib.rs").to_string_lossy().to_string(),
            "tests/test_lib.rs".to_string(),
            "/tmp/mixmod-state/projects/app/worker-task.json".to_string(),
            "../escape.rs".to_string(),
            "./src/lib.rs".to_string(),
        ],
    );

    assert_eq!(files, vec!["src/lib.rs", "tests/test_lib.rs"]);
}

#[test]
fn live_control_message_drops_mixmod_task_reference() {
    let message = sanitize_live_control_message(
        "Context overflow with no diff yet. Start from worker-task.json, then edit src/lib.rs.",
        &["src/lib.rs".to_string()],
    );

    assert!(!message.contains("worker-task.json"));
    assert!(message.contains("src/lib.rs"));
    assert!(message.contains("git diff --stat"));
}

#[test]
fn parse_feedback_json_normalizes_object_exact_edits() {
    let parsed = parse_feedback_json(
        r#"{
                "handoff":"guided",
                "expect_patch":true,
                "worker_turn_shape":"small_patch_slice",
                "exact_edits":[
                    {
                        "file":"src/lib.rs",
                        "symbol":"configure",
                        "instruction":"Add the new option to the returned metadata."
                    }
                ]
            }"#,
    )
    .unwrap();

    assert_eq!(
        get_string_array(&parsed, "exact_edits"),
        vec!["In src/lib.rs, update configure: Add the new option to the returned metadata."]
    );
}

#[test]
fn approval_with_required_checks_is_inconsistent() {
    let feedback = json!({
        "action": "approve",
        "message_to_worker": "Looks complete.",
        "required_checks": ["cargo test -p mixmod"],
        "deferred_checks": [],
        "risk": "none"
    });

    let rejection = approval_consistency_rejection(&feedback).unwrap();

    assert!(rejection.contains("action=approve"));
    assert!(rejection.contains("required_checks"));
}

#[test]
fn approval_with_deferred_checks_or_gate_is_inconsistent() {
    let feedback = json!({
        "action": "approve",
        "message_to_worker": "Looks complete.",
        "required_checks": [],
        "deferred_checks": ["cargo test -p mixmod"],
        "completion_gate": "focused test must pass",
        "risk": "none"
    });

    let rejection = approval_consistency_rejection(&feedback).unwrap();

    assert!(rejection.contains("deferred_checks"));
    assert!(rejection.contains("completion_gate"));
}

#[test]
fn clean_approval_has_no_consistency_rejection() {
    let feedback = json!({
        "action": "approve",
        "message_to_worker": "Focused tests passed in artifacts.",
        "required_checks": [],
        "deferred_checks": [],
        "risk": "none"
    });

    assert!(approval_consistency_rejection(&feedback).is_none());
}

#[test]
fn approval_consistency_repair_rejects_stop() {
    let feedback = json!({
        "action": "stop",
        "message_to_worker": "No further worker attempts.",
        "required_checks": []
    });

    assert!(!approval_consistency_repair_is_accepted(&feedback));
}

#[test]
fn inconsistent_approval_fallback_becomes_verification_revision() {
    let feedback = json!({
        "action": "approve",
        "worker_mode": "context_focus",
        "focus_files": ["src/lib.rs"],
        "required_checks": ["cargo test -p mixmod"],
        "deferred_checks": ["cargo test --doc"],
        "completion_gate": "focused behavior check must pass"
    });

    let revision = verification_revision_for_inconsistent_approval(&feedback);

    assert_eq!(get_str(&revision, "action"), Some("revise"));
    assert_eq!(get_str(&revision, "patch_decision"), Some("revise_current"));
    assert_eq!(get_str(&revision, "worker_mode"), Some("context_focus"));
    assert_eq!(
        get_string_array(&revision, "focus_files"),
        vec!["src/lib.rs"]
    );
    assert_eq!(
        get_string_array(&revision, "required_checks"),
        vec![
            "Completion gate: focused behavior check must pass",
            "cargo test --doc",
            "cargo test -p mixmod"
        ]
    );
}

#[test]
fn object_style_initial_metadata_seed_does_not_need_repair() {
    let parsed = parse_feedback_json(
            r#"{
                "handoff":"guided",
                "expect_patch":true,
                "worker_turn_shape":"small_patch_slice",
                "turn_goal":"add public field metadata options",
                "files":["src/helper.py"],
                "exact_edits":[
                    {
                        "file":"src/helper.py",
                        "symbol":"field_options",
                        "instruction":"Near the line containing \"def field_options(\", add keyword parameters for one metadata seed and store their values in the returned metadata/options dict. Do not implement validation, packing, unpacking, aliases, prefix behavior, rename behavior, serialization, deserialization, or tests in this slice."
                    }
                ]
            }"#,
        )
        .unwrap();

    assert!(!worker_brief_needs_small_slice_repair(
        &parsed,
        &small_slice_guidance()
    ));
}

#[test]
fn initial_api_seed_with_flatten_option_names_does_not_need_repair() {
    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "worker_turn_shape": "small_patch_slice",
        "exact_edits": [
            "In mashumaro/helper.py, update symbol field_options near the line containing \"def field_options\" to accept flatten=False, flatten_prefix=None, and flatten_rename=None, then include exactly those three values in the returned metadata/options structure following the existing field_options style."
        ]
    });

    assert!(!worker_brief_needs_small_slice_repair(
        &brief,
        &small_slice_guidance()
    ));
}

#[test]
fn broad_small_slice_revision_feedback_does_not_need_repair() {
    let feedback = json!({
        "action": "revise",
        "worker_turn_shape": "small_patch_slice",
        "exact_edits": [
            "In builder.py, near the field loop, support flatten by adding pack and unpack behavior for nested dataclasses."
        ]
    });

    assert!(!supervisor_feedback_needs_revision_slice_repair(
        &feedback,
        &small_slice_guidance()
    ));
}

#[test]
fn complex_non_small_revision_feedback_without_worker_profile_does_not_need_repair() {
    let feedback = json!({
        "action": "revise",
        "message_to_worker": "Implement generated pack/unpack alias validation for nested metadata.",
        "focus_files": ["src/builder.py"]
    });

    assert!(!supervisor_feedback_needs_revision_slice_repair(
        &feedback,
        &WorkerSupervisorGuidance::default()
    ));
}

#[test]
fn non_small_revision_feedback_for_small_slice_worker_needs_repair() {
    let feedback = json!({
        "action": "revise",
        "message_to_worker": "Implement generated pack/unpack alias validation for nested metadata.",
        "focus_files": ["src/builder.py"]
    });

    assert!(supervisor_feedback_needs_revision_slice_repair(
        &feedback,
        &small_slice_guidance()
    ));
}

#[test]
fn atomic_small_slice_revision_feedback_does_not_need_repair() {
    let feedback = json!({
        "action": "revise",
        "worker_turn_shape": "small_patch_slice",
        "exact_edits": [
            "In builder.py, near the line containing `for field in fields`, collect flatten=True field names into flattened_fields."
        ]
    });

    assert!(!supervisor_feedback_needs_revision_slice_repair(
        &feedback,
        &small_slice_guidance()
    ));
}

#[test]
fn anchored_revision_edit_with_negative_limits_does_not_need_repair() {
    let feedback = json!({
        "action": "revise",
        "worker_turn_shape": "small_patch_slice",
        "exact_edits": [
            "In mashumaro/core/meta/code/builder.py, in symbol _add_pack_method_lines near the line containing \"packers = {}\", handle only metadata.get(\"flatten\") in the packing/kwargs-building branch: for a flattened field, update kwargs with the packed child dict instead of assigning kwargs[fname]; do not add unpacking, validation, prefix, rename, alias-collision, or test edits in this turn."
        ]
    });

    assert!(!supervisor_feedback_needs_revision_slice_repair(
        &feedback,
        &small_slice_guidance()
    ));
}

#[test]
fn revision_repair_must_preserve_single_focus_file() {
    let previous = json!({
        "action": "revise",
        "focus_files": ["mashumaro/core/meta/code/builder.py"],
        "exact_edits": [
            "In mashumaro/core/meta/code/builder.py, implement a serialization-only flatten slice."
        ]
    });
    let repaired = json!({
        "action": "revise",
        "focus_files": ["mashumaro/helper.py"],
        "exact_edits": [
            "In mashumaro/helper.py, remove flatten_prefix and flatten_rename from field_options."
        ]
    });

    assert!(!revision_repair_preserves_focus(&previous, &repaired));
}

#[test]
fn revision_repair_can_keep_single_focus_file() {
    let previous = json!({
        "action": "revise",
        "focus_files": ["mashumaro/core/meta/code/builder.py"],
        "exact_edits": [
            "In mashumaro/core/meta/code/builder.py, implement a serialization-only flatten slice."
        ]
    });
    let repaired = json!({
        "action": "revise",
        "focus_files": ["mashumaro/core/meta/code/builder.py"],
        "exact_edits": [
            "In mashumaro/core/meta/code/builder.py, near the line containing \"packers = {}\", collect field names with metadata flatten=True."
        ]
    });

    assert!(revision_repair_preserves_focus(&previous, &repaired));
}

#[test]
fn broad_worker_brief_repair_gets_structural_rejection_reason() {
    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "worker_turn_shape": "default",
        "message_to_worker": "Implement the feature."
    });

    let reason = worker_brief_repair_rejection_reason(Some(&brief), &small_slice_guidance());

    assert!(reason.contains("small_patch_slice shape"));
    assert!(reason.contains("one concrete source edit"));
}

#[test]
fn changed_focus_revision_repair_gets_structural_rejection_reason() {
    let previous = json!({
        "action": "revise",
        "worker_turn_shape": "small_patch_slice",
        "focus_files": ["src/builder.rs"],
        "exact_edits": ["In src/builder.rs, make one serialization edit."]
    });
    let repaired = json!({
        "action": "revise",
        "worker_turn_shape": "small_patch_slice",
        "focus_files": ["src/helper.rs"],
        "exact_edits": ["In src/helper.rs, make one helper edit."]
    });

    let reason = supervisor_feedback_repair_rejection_reason(
        &previous,
        Some(&repaired),
        &small_slice_guidance(),
    );

    assert!(reason.contains("changed away from the previous single focus file"));
}

#[test]
fn blocked_worker_brief_does_not_need_repair() {
    let brief = json!({
        "handoff": "blocked",
        "expect_patch": false,
        "message_to_worker": "Cannot proceed."
    });

    assert!(!worker_brief_needs_small_slice_repair(
        &brief,
        &small_slice_guidance()
    ));
}
