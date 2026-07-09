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

    assert!(prompt.contains("worker_mode=continue to keep the same worker session"));
    assert!(prompt.contains("worker_mode=context_focus to start a new worker session"));
    assert!(prompt.contains("worker session as context-saturated"));
    assert!(prompt.contains("prefer worker_mode=context_focus"));
    assert!(prompt.contains("one concrete source edit"));
    assert!(prompt.contains("patch_decision"));
    assert!(prompt.contains("revise_previous"));
    assert!(prompt.contains("Mixmod will restore the previous candidate worktree"));
    assert!(prompt.contains("focused follow-up edit to apply after rollback"));
    assert!(prompt.contains("Do not ask the worker to inspect Mixmod state"));
    assert!(prompt.contains("previous worker context is discarded"));
    assert!(prompt.contains("Do not implement code. Do not edit files."));
    assert!(prompt.contains("Do not ask the user for approval."));
    assert!(prompt.contains("worker-model guidance as part of the supervisor decision contract"));
    assert!(prompt.contains("a broad revise is the wrong decision"));
    assert!(prompt.contains("use stop with a clear risk instead of sending a broad revision"));
    assert!(
        prompt.contains(
            "Prefer revise after failed, empty, distracted, or incomplete worker attempts"
        )
    );
    assert!(prompt.contains("generated keys, aliases, field names"));
    assert!(prompt.contains("raw names and configured aliases"));
    assert!(prompt.contains("focused source repair or regression check"));
    assert!(prompt.contains("not merely because the latest worker turn created a non-empty diff"));
    assert!(prompt.contains("You own final task completeness"));
    assert!(prompt.contains("classify whether the accumulated patch changes runtime behavior"));
    assert!(prompt.contains("task-derived checks"));
    assert!(prompt.contains("verification-focused worker turn"));
    assert!(prompt.contains("worker-brief.json used worker_turn_shape=small_patch_slice"));
    assert!(prompt.contains("set worker_turn_shape=small_patch_slice with the next narrow"));
    assert!(prompt.contains("worker-brief.json used worker_turn_shape=bounded_feature_slice"));
    assert!(prompt.contains("judge whether the previous worker slice was too much"));
    assert!(prompt.contains("previous slices may now be too small"));
    assert!(prompt.contains("needed supervisor control"));
    assert!(prompt.contains("A corrective small_patch_slice is recovery, not promotion"));
    assert!(prompt.contains("not a new bundle of validation"));
    assert!(prompt.contains("first useful end-to-end behavior path"));
    assert!(prompt.contains("Use bounded_feature_slice only when the selected worker guidance"));
    assert!(prompt.contains("keep or enlarge the next slice as bounded_feature_slice"));
    assert!(prompt.contains("broaden only the anchored source behavior inside that shape"));
    assert!(prompt.contains(
        "promotion means one coherent anchored source behavior inside small_patch_slice"
    ));
    assert!(prompt.contains("exact_edits"));
    assert!(prompt.contains("Make the next slice one behavior only"));
    assert!(prompt.contains("Treat exact_edits as a queue"));
    assert!(prompt.contains("put one source edit first"));
    assert!(prompt.contains("current accumulated worktree.patch"));
    assert!(prompt.contains("Preserve useful existing edits"));
    assert!(prompt.contains("local transformation near one anchor"));
    assert!(prompt.contains("compile-driven repair slice"));
    assert!(prompt.contains("edit_packet"));
    assert!(prompt.contains("source_snippets"));
    assert!(prompt.contains("exact symbols plus a literal nearby code anchor"));
    assert!(prompt.contains("literal nearby code anchor"));
    assert!(prompt.contains("Stop does not permit direct supervisor editing."));
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
