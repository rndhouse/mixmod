use super::super::*;

#[test]
fn supervisor_feedback_prompt_explains_worker_session_modes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let prompt = supervisor_feedback_prompt(
        root,
        &[root.join("missing-report.md")],
        "decide",
        &WorkerSupervisorGuidance::default(),
    )
    .unwrap();

    assert!(prompt.contains("Core review contract"));
    assert!(prompt.contains("worker_mode=continue reuses the current worker session"));
    assert!(prompt.contains("worker_mode=context_focus starts a fresh worker session"));
    assert!(prompt.contains("patch_decision"));
    assert!(prompt.contains("revise_previous"));
    assert!(prompt.contains("Workspace access is for supervision, not implementation"));
    assert!(prompt.contains("git status"));
    assert!(prompt.contains("Do not author task-solving source edits"));
    assert!(prompt.contains("Do not ask the user for approval."));
    assert!(prompt.contains("Treat supervisor input tokens as scarce"));
    assert!(prompt.contains("For ordinary worker-turn review"));
    assert!(prompt.contains("start with task context, compact metadata, and changes.patch"));
    assert!(prompt.contains("Prefer latest-turn evidence first"));
    assert!(prompt.contains("Avoid opening worktree.patch unless approval"));
    assert!(prompt.contains("do not inspect more artifacts, logs, or diff content"));
    assert!(prompt.contains("Approve only when the accumulated patch appears to satisfy"));
    assert!(prompt.contains("Before approving, inspect task.json and enough accumulated state"));
    assert!(prompt.contains("false approval as a terminal correctness failure"));
    assert!(prompt.contains("main requested behavior or a likely edge case"));
    assert!(prompt.contains("Revise when a useful worker path remains"));
    assert!(prompt.contains("Stop only for a blocked or inconclusive worker result"));
    assert!(prompt.contains("The worker owns implementation"));
    assert!(prompt.contains("Prefer patch_decision for rollback control"));
    assert!(prompt.contains("Put only repo source/test paths in focus_files"));
    assert!(prompt.contains("exact_edits"));
    assert!(!prompt.contains("Context-pressure context"));
    assert!(!prompt.contains("Small-patch slice context"));
    assert!(!prompt.contains("Patch checkpoint context"));
}

#[test]
fn supervisor_feedback_prompt_adds_situational_context_from_artifacts() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let worker_brief = root.join(WORKER_BRIEF_JSON);
    let metrics = root.join(METRICS_JSON);
    let loop_summary = root.join(SUPERVISION_LOOP_SUMMARY_JSON);
    let tool_events = root.join(TOOL_EVENTS_JSONL);
    let patch_comparison = root.join(PATCH_COMPARISON);
    atomic_write(
        &worker_brief,
        br#"{"worker_turn_shape":"small_patch_slice"}"#,
    )
    .unwrap();
    atomic_write(
        &metrics,
        br#"{"context_overflow_count":1,"worker_session_token_peak":27000,"interrupted_by_supervisor":true,"supervisor_control_events":[{"action":"interrupt_context_focus"}]}"#,
    )
    .unwrap();
    atomic_write(
        &loop_summary,
        br#"{"small_patch_slice_nonempty_delta_streak":2,"turns":[{"worker_turn_shape":"small_patch_slice","context_overflow_count":1,"worker_session_token_peak":27000}]}"#,
    )
    .unwrap();
    atomic_write(&tool_events, b"").unwrap();
    atomic_write(&patch_comparison, b"{}").unwrap();

    let prompt = supervisor_feedback_prompt(
        root,
        &[
            worker_brief,
            metrics,
            loop_summary,
            tool_events,
            patch_comparison,
        ],
        "decide",
        &WorkerSupervisorGuidance::default(),
    )
    .unwrap();

    assert!(prompt.contains("Use tool-events.jsonl as command/tool-call evidence"));
    assert!(prompt.contains("Small-patch slice context"));
    assert!(prompt.contains("Context-pressure context"));
    assert!(prompt.contains("Live-control context"));
    assert!(prompt.contains("Slice-sizing context"));
    assert!(prompt.contains("Patch checkpoint context"));
    assert!(prompt.contains("Mixmod restores that candidate before the next worker turn"));
}

#[test]
fn supervisor_feedback_prompt_lists_artifacts_without_embedding_contents() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let report = root.join(REPORT_MD);
    let patch = root.join(WORKTREE_PATCH);
    let tool_events = root.join(TOOL_EVENTS_JSONL);
    atomic_write(&report, b"SECRET_ARTIFACT_BODY_SHOULD_NOT_BE_EMBEDDED").unwrap();
    atomic_write(&patch, b"diff --git a/src/lib.rs b/src/lib.rs\n").unwrap();
    atomic_write(
        &tool_events,
        br#"{"type":"tool_use","part":{"tool":"bash"}}"#,
    )
    .unwrap();

    let prompt = supervisor_feedback_prompt(
        root,
        &[report.clone(), patch.clone(), tool_events.clone()],
        "decide",
        &WorkerSupervisorGuidance::default(),
    )
    .unwrap();

    assert!(prompt.contains("Artifact index:"));
    assert!(prompt.contains("Inspect the listed artifact files directly"));
    assert!(prompt.contains("report.md"));
    assert!(prompt.contains("compact worker-run summary"));
    assert!(prompt.contains("worktree.patch"));
    assert!(prompt.contains("accumulated current repository diff"));
    assert!(prompt.contains("tool-events.jsonl"));
    assert!(prompt.contains("worker tool-call events extracted from structured output"));
    assert!(!prompt.contains("SECRET_ARTIFACT_BODY_SHOULD_NOT_BE_EMBEDDED"));
    assert!(!prompt.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    assert!(!prompt.contains("\"tool\":\"bash\""));
}

#[test]
fn supervisor_feedback_repair_prompt_preserves_accumulated_work() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let artifact = root.join("worktree.patch");
    atomic_write(
        &artifact,
        br#"diff --git a/mashumaro/helper.py b/mashumaro/helper.py
--- a/mashumaro/helper.py
+++ b/mashumaro/helper.py
@@ -1,1 +1,3 @@
+flatten_prefix
+flatten_rename
"#,
    )
    .unwrap();
    let previous = json!({
        "action": "revise",
        "focus_files": ["mashumaro/core/meta/code/builder.py"],
        "exact_edits": [
            "In mashumaro/core/meta/code/builder.py, implement serialization-only flatten support."
        ]
    });

    let prompt = supervisor_feedback_repair_prompt(
        root,
        &[artifact],
        &WorkerSupervisorGuidance::default(),
        &previous,
    )
    .unwrap();

    assert!(prompt.contains("Preserve the previous feedback's intended target behavior"));
    assert!(prompt.contains("do not rewind to an earlier completed slice"));
    assert!(prompt.contains("Treat useful accumulated worktree.patch changes as context to keep"));
    assert!(
        prompt.contains("Write the repaired exact edit from the current accumulated patch state")
    );
    assert!(prompt.contains("Do not say to continue from an earlier file-only slice"));
    assert!(prompt.contains("edit_packet or source_snippets"));
    assert!(prompt.contains("local transformation near one anchor"));
    assert!(prompt.contains("If previous feedback named one focus file"));
    assert!(prompt.contains("exact_edits must be an array with exactly one string item"));
    assert!(prompt.contains("Do not invent a different file/symbol pair"));
    assert!(prompt.contains("Artifact index:"));
    assert!(prompt.contains("worktree.patch"));
    assert!(!prompt.contains("flatten_prefix"));
    assert!(!prompt.contains("flatten_rename"));
}

#[test]
fn supervisor_prompts_include_selected_worker_model_guidance() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let guidance = MixmodConfig::default().worker_supervisor_guidance();
    let feedback_prompt =
        supervisor_feedback_prompt(root, &[root.join("missing-report.md")], "decide", &guidance)
            .unwrap();

    assert!(feedback_prompt.contains("Supervisor-only worker-model guidance"));
    assert!(feedback_prompt.contains("Do not copy every bullet to the worker"));
    assert!(feedback_prompt.contains("Treat applicable bullets as binding"));
    assert!(feedback_prompt.contains("global environments"));
    assert!(feedback_prompt.contains("end-to-end semantics across slices"));
    assert!(feedback_prompt.contains("task-derived probes or focused tests"));
    assert!(feedback_prompt.contains("entry point actually uses it"));

    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Checkout",
  "instructions": "Fix totals.",
  "tests": ["python -m unittest -q"]
}"#,
    )
    .unwrap();

    let brief_prompt =
        supervisor_worker_brief_prompt(root, &task, &guidance, SupervisorInitMode::Compact)
            .unwrap();
    assert!(brief_prompt.contains("Supervisor-only worker-model guidance"));
    assert!(brief_prompt.contains("reasoning before editing"));
    assert!(brief_prompt.contains("large effective context"));
    assert!(brief_prompt.contains("split broad tasks into small concrete source slices"));
    assert!(brief_prompt.contains("worker_turn_shape=small_patch_slice"));
    assert!(brief_prompt.contains("one immediate source edit"));
    assert!(brief_prompt.contains("current accumulated patch"));
    assert!(brief_prompt.contains("one literal anchor plus the smallest local transformation"));
    assert!(brief_prompt.contains("compile-driven repair instruction"));
    assert!(brief_prompt.contains("alias/key generated-code repairs"));
    assert!(brief_prompt.contains("raw field names and resolved aliases"));
    assert!(brief_prompt.contains("use worker_turn_shape=small_patch_slice by default"));
    assert!(brief_prompt.contains("switching this profile to bounded_feature_slice"));
    assert!(
        brief_prompt.contains("make the exact source edit more coherent inside small_patch_slice")
    );
    assert!(brief_prompt.contains("multiple clean small_patch_slice revisions"));
    assert!(brief_prompt.contains("prioritize the first useful behavior path"));
    assert!(brief_prompt.contains("context overflow"));
    assert!(brief_prompt.contains("worker_mode=context_focus"));
    assert!(brief_prompt.contains("Select only relevant points"));
    assert!(brief_prompt.contains("worker-model guidance as handoff constraints"));
    assert!(brief_prompt.contains("satisfy that contract in the first JSON turn"));
    assert!(brief_prompt.contains("exact_edits as exactly one string source edit"));
    assert!(brief_prompt.contains("base path plus modifiers"));
    assert!(brief_prompt.contains("one modifier family per later slice"));
}

#[test]
fn supervisor_prompts_include_openrouter_glm_worker_guidance() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = MixmodConfig::default();
    ModelOverrides::new(None, Some("openrouter/z-ai/glm-5.2".to_string()))
        .apply_to_config(&mut config)
        .unwrap();
    let guidance = config.worker_supervisor_guidance();

    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Parser defaults",
  "instructions": "Add parser support for default args.",
  "tests": []
}"#,
    )
    .unwrap();

    let brief_prompt =
        supervisor_worker_brief_prompt(root, &task, &guidance, SupervisorInitMode::Compact)
            .unwrap();

    assert!(brief_prompt.contains("Supervisor-only worker-model guidance"));
    assert!(brief_prompt.contains("openrouter/z-ai/glm-5.2"));
    assert!(brief_prompt.contains("over-investigate"));
    assert!(brief_prompt.contains("resolve the implementation route"));
    assert!(brief_prompt.contains("trust that route"));
    assert!(brief_prompt.contains("worker_turn_shape=bounded_feature_slice"));
    assert!(brief_prompt.contains("patch-first"));
    assert!(brief_prompt.contains("toolchain archaeology"));
    assert!(brief_prompt.contains("current accumulated patch"));
    assert!(brief_prompt.contains("end-to-end behavior"));
}

#[test]
fn supervisor_review_artifacts_include_task_and_handoff_context() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let default_dir = root.join("default");
    let worker_dir = root.join("worker");
    fs::create_dir_all(&default_dir).unwrap();
    fs::create_dir_all(&worker_dir).unwrap();
    for name in [TASK_JSON, WORKER_BRIEF_JSON, WORKER_TASK_JSON] {
        atomic_write(&default_dir.join(name), b"{}").unwrap();
    }
    for name in RUN_COMPACT_ARTIFACTS {
        atomic_write(&worker_dir.join(name), b"{}").unwrap();
    }

    let paths = supervisor_review_artifact_paths(&default_dir, &worker_dir)
        .into_iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        &paths[..3],
        &[TASK_JSON, WORKER_BRIEF_JSON, WORKER_TASK_JSON]
    );
    assert!(paths.contains(&WORKTREE_PATCH.to_string()));
    assert!(paths.contains(&CHANGES_PATCH.to_string()));
    assert!(paths.contains(&TOOL_EVENTS_JSONL.to_string()));
}
