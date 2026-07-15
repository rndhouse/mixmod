use std::path::Path;

use serde_json::json;

use crate::{
    LiveSupervisionConfig, LiveWorkerSnapshot, WorkerBackendSlotTelemetry, WorkerBackendTelemetry,
    WorkerSupervisorGuidance, get_str, get_string_array,
};

use super::live::{
    live_supervision_snapshot_should_check, normalize_live_control_focus_files,
    sanitize_live_control_message, snapshot_for_live_supervisor_prompt,
};
use super::normalize::parse_feedback_json;
use super::turns::{
    approval_consistency_rejection, approval_consistency_repair_is_accepted,
    verification_revision_for_inconsistent_approval,
};
use super::*;

#[test]
fn live_supervision_waits_when_worker_has_fresh_delta() {
    let snapshot = LiveWorkerSnapshot {
        elapsed_ms: 300_000,
        new_delta_bytes: 120,
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
fn live_supervisor_prompt_snapshot_redacts_artifact_paths() {
    let snapshot = LiveWorkerSnapshot {
        out_dir: "/tmp/mixmod-state/projects/app/runs/run-1/worker-runs/proposal".to_string(),
        task_path: "/tmp/mixmod-state/projects/app/runs/run-1/worker-task.json".to_string(),
        stdout_log_path:
            "/tmp/mixmod-state/projects/app/runs/run-1/worker-runs/proposal/logs/opencode.stdout.txt"
                .to_string(),
        stderr_log_path:
            "/tmp/mixmod-state/projects/app/runs/run-1/worker-runs/proposal/logs/opencode.stderr.txt"
                .to_string(),
        tool_events_path:
            "/tmp/mixmod-state/projects/app/runs/run-1/worker-runs/proposal/tool-events.jsonl"
                .to_string(),
        worker_instruction_excerpt:
            "Original task instructions: Add a flatten option to field_options.".to_string(),
        live_control_check_index: 3,
        live_control_check_limit: 3,
        elapsed_ms: 130_000,
        context_overflow_count: 1,
        worker_backend_telemetry: Some(WorkerBackendTelemetry {
            provider: "llama_server".to_string(),
            available: true,
            captured_at: "2026-07-10T10:35:00Z".to_string(),
            ctx_size: Some(32768),
            requests_processing: Some(1),
            requests_deferred: Some(0),
            tokens_max_observed: Some(27142),
            active_slots: vec![WorkerBackendSlotTelemetry {
                id: 0,
                ctx_size: Some(32768),
                is_processing: true,
                decoded_tokens: Some(814),
                remaining_tokens: Some(-1),
            }],
            error: None,
        }),
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
    assert!(prompt.contains("Available actions:"));
    assert!(prompt.contains("Worker session context economics:"));
    assert!(prompt.contains("Cached input tokens are cheaper than uncached input"));
    assert!(prompt.contains("large cached session can still dominate cost and latency"));
    assert!(prompt.contains("interrupt_context_focus starts a fresh worker session"));
    assert!(prompt.contains("spend uncached input rereading files"));
    assert!(prompt.contains("worker_session_token_peak/context pressure are modest"));
    assert!(prompt.contains("phase boundary where the next slice can be restated compactly"));
    assert!(prompt.contains("right next decision requires patch_decision"));
    assert!(prompt.contains("Base the action on the live evidence"));
    assert!(prompt.contains("Do not assume an intervention is required"));
    assert!(prompt.contains("worker_backend_telemetry"));
    assert!(prompt.contains("tokens_max_observed"));
    assert!(prompt.contains("27142"));
    assert!(prompt.contains("stdout_log_path"));
    assert!(prompt.contains("stderr_log_path"));
    assert!(prompt.contains("opencode.stdout.txt"));
    assert!(prompt.contains("opencode.stderr.txt"));
    assert!(prompt.contains("tool_events_path"));
    assert!(prompt.contains("tool-events.jsonl"));
    assert!(prompt.contains("If you need detailed stdout, stderr, or tool-call history"));
    assert!(prompt.contains("Do not invent a different cleanup, bug, or objective"));
    assert!(prompt.contains("abort_worker_turn"));
    assert!(!prompt.contains("worker_context_pressure"));
    assert!(!prompt.contains("prefer an interrupt"));
    assert!(!prompt.contains("stdout_tail"));
    assert!(!prompt.contains("stderr_tail"));
    assert!(!prompt.contains("recent_tool_events"));
    assert!(!prompt.contains("Prefer this after"));
    assert!(!prompt.contains("prefer interrupt_context_focus"));
    assert!(!prompt.contains("\"out_dir\": \"/tmp/mixmod-state"));
    assert!(!prompt.contains("\"task_path\": \"/tmp/mixmod-state"));
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
    assert!(!message.contains("git diff --stat"));
}

#[test]
fn parse_feedback_json_normalizes_object_exact_edits() {
    let parsed = parse_feedback_json(
        r#"{
                "handoff":"guided",
                "expect_patch":true,
                "worker_turn_shape":"patch_request",
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
