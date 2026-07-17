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
        &SupervisorContextTelemetry::default(),
        DefaultStrategyMode::SupervisedWorker,
    )
    .unwrap();

    assert!(prompt.contains("Core review contract"));
    assert!(prompt.contains("Worker session context economics:"));
    assert!(prompt.contains("Approval contract policy:"));
    assert!(prompt.contains("approval_contract rows derived from the original task"));
    assert!(prompt.contains("required behavior, default/disabled behavior, boundary cases"));
    assert!(prompt.contains("Do not copy a fixed checklist across tasks"));
    assert!(prompt.contains("approval_possible_after_verification"));
    assert!(prompt.contains("ready_to_approve"));
    assert!(prompt.contains("\"approval_contract\""));
    assert!(prompt.contains("\"approval_state\""));
    assert!(prompt.contains("\"approval_blocker\""));
    assert!(prompt.contains("worker_mode=continue reuses useful recent file/tool context"));
    assert!(prompt.contains("worker_mode=context_focus starts a fresh worker session"));
    assert!(prompt.contains("Cached input tokens are cheaper than uncached input"));
    assert!(prompt.contains("large cached session can still dominate cost and latency"));
    assert!(prompt.contains("starts a fresh worker session on the same source tree"));
    assert!(prompt.contains("spend uncached input rereading files"));
    assert!(prompt.contains("worker_session_token_peak/context pressure are modest"));
    assert!(prompt.contains("phase boundary where the next slice can be restated compactly"));
    assert!(
        prompt.contains("patch_decision=accept_current_baseline with worker_mode=context_focus")
    );
    assert!(prompt.contains("clean active diff and fresh session context"));
    assert!(prompt.contains("patch_decision"));
    assert!(prompt.contains("accept_current_baseline"));
    assert!(prompt.contains("revise_previous"));
    assert!(prompt.contains("Workspace access is for supervision, not implementation"));
    assert!(prompt.contains("git status"));
    assert!(prompt.contains("Do not author task-solving source edits"));
    assert!(prompt.contains("Do not ask the user for approval."));
    assert!(prompt.contains("Treat supervisor input tokens as scarce"));
    assert!(prompt.contains("Supervisor context telemetry:"));
    assert!(prompt.contains("context_recommendation"));
    assert!(prompt.contains("compact_now"));
    assert!(prompt.contains("compact_after_next_worker"));
    assert!(prompt.contains("worker_turn_shape=\"planning_probe\""));
    assert!(prompt.contains("After a planning_probe result"));
    assert!(prompt.contains("fresh worker session"));
    assert!(prompt.contains("For ordinary worker-turn review"));
    assert!(prompt.contains("start with task context, receipt/report, review-signals.json"));
    assert!(prompt.contains("Prefer latest-turn evidence first"));
    assert!(prompt.contains("Avoid opening worktree.patch or running broad git diff"));
    assert!(prompt.contains("Treat review-signals.json as the routing layer"));
    assert!(prompt.contains("do not inspect more artifacts, logs, or diff content"));
    assert!(prompt.contains("For generated-output diffs"));
    assert!(prompt.contains("Avoid opening whole generated files"));
    assert!(prompt.contains("transient tool sidecars"));
    assert!(prompt.contains("Approve only when the current source state appears to satisfy"));
    assert!(prompt.contains("Approval requires deterministic artifact/source/check evidence"));
    assert!(prompt.contains("Before approving, inspect task.json and enough source/diff state"));
    assert!(prompt.contains("false approval as a terminal correctness failure"));
    assert!(prompt.contains("approval_contract evidence is missing"));
    assert!(prompt.contains("main requested behavior or a likely edge case"));
    assert!(prompt.contains("Revise when a useful worker path remains"));
    assert!(prompt.contains("Stop only for a blocked or inconclusive worker result"));
    assert!(prompt.contains("The worker owns implementation"));
    assert!(prompt.contains("Prefer patch_decision for checkpoint control"));
    assert!(prompt.contains("Use patch_decision=accept_current_baseline"));
    assert!(prompt.contains("Put only repo source/test paths in focus_files"));
    assert!(prompt.contains("exact_edits"));
    assert!(!prompt.contains("Context-pressure context"));
    assert!(!prompt.contains("Patch request context"));
    assert!(!prompt.contains("Patch checkpoint context"));
}

#[test]
fn worker_build_supervisor_fix_feedback_prompt_prefers_direct_correction() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let prompt = supervisor_feedback_prompt(
        root,
        &[root.join("missing-report.md")],
        "decide",
        &WorkerSupervisorGuidance::default(),
        &SupervisorContextTelemetry::default(),
        DefaultStrategyMode::WorkerBuildSupervisorFix,
    )
    .unwrap();

    assert!(prompt.contains("Strategy mode: worker-build-supervisor-fix"));
    assert!(prompt.contains("Use the worker for construction"));
    assert!(prompt.contains("Use action=take_over only for surgical correction"));
    assert!(prompt.contains("Before action=revise, classify the next request"));
    assert!(prompt.contains("named residual defects"));
    assert!(prompt.contains("Choose revise when the next step needs broad search"));
    assert!(prompt.contains("Before action=take_over, confirm the direct_plan can be executed"));
    assert!(prompt.contains("Put broad or command-based checks in a later worker"));
    assert!(prompt.contains("Corrections can appear before every broad task area is complete"));
    assert!(prompt.contains("\"action\":\"approve|revise|take_over|stop\""));
    assert!(prompt.contains("\"takeover_reason\""));
    assert!(prompt.contains("\"direct_plan\""));
    assert!(!prompt.contains("\"delegation_decision\""));
}

#[test]
fn worker_build_supervisor_fix_debug_prompt_requires_delegation_decision() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let prompt = supervisor_feedback_prompt_with_debug_profile_fit(
        root,
        &[root.join("missing-report.md")],
        "decide",
        &WorkerSupervisorGuidance::default(),
        &SupervisorContextTelemetry::default(),
        DefaultStrategyMode::WorkerBuildSupervisorFix,
    )
    .unwrap();

    assert!(prompt.contains("Debug delegation-decision audit"));
    assert!(prompt.contains("delegation_decision.next_owner"));
    assert!(prompt.contains("delegation_decision.work_type"));
    assert!(
        prompt.contains(
            "why the next step belongs with the normal worker or a takeover worker patch"
        )
    );
    assert!(prompt.contains("\"delegation_decision\""));
    assert!(prompt.contains("\"worker_fit\""));
    assert!(prompt.contains("\"direct_fit\""));
}

#[test]
fn supervisor_feedback_prompt_adds_situational_context_from_artifacts() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let worker_brief = root.join(WORKER_BRIEF_JSON);
    let review_signals = root.join(REVIEW_SIGNALS_JSON);
    let loop_summary = root.join(SUPERVISION_LOOP_SUMMARY_JSON);
    let patch_comparison = root.join(PATCH_COMPARISON);
    let changes_patch = root.join(CHANGES_PATCH);
    atomic_write(&worker_brief, br#"{"worker_turn_shape":"patch_request"}"#).unwrap();
    atomic_write(
        &review_signals,
        br#"{"context_overflow_count":1,"worker_session_token_peak":27000,"interrupted_by_supervisor":true,"supervisor_control_action":"interrupt_context_focus","tool_event_count":3}"#,
    )
    .unwrap();
    atomic_write(
        &loop_summary,
        br#"{"patch_request_nonempty_delta_streak":2,"turns":[{"worker_turn_shape":"patch_request","context_overflow_count":1,"worker_session_token_peak":27000}]}"#,
    )
    .unwrap();
    atomic_write(&patch_comparison, b"{}").unwrap();
    atomic_write(&changes_patch, b"").unwrap();

    let prompt = supervisor_feedback_prompt(
        root,
        &[
            worker_brief,
            review_signals,
            loop_summary,
            patch_comparison,
            changes_patch,
        ],
        "decide",
        &WorkerSupervisorGuidance::default(),
        &SupervisorContextTelemetry::default(),
        DefaultStrategyMode::SupervisedWorker,
    )
    .unwrap();

    assert!(prompt.contains("Use tool-events.jsonl only as command/tool-call evidence"));
    assert!(prompt.contains("Patch request context"));
    assert!(prompt.contains("No-diff patch-request context"));
    assert!(prompt.contains("Context-pressure context"));
    assert!(prompt.contains("Live-control context"));
    assert!(prompt.contains("Slice-sizing context"));
    assert!(prompt.contains("Patch checkpoint context"));
    assert!(prompt.contains("baseline candidate before the next slice"));
    assert!(prompt.contains("The latest changes.patch appears empty"));
    assert!(prompt.contains("prior request as likely too broad or under-anchored"));
    assert!(prompt.contains("shrink at least one dimension"));
    assert!(prompt.contains("Do not resend the same broad patch_request"));
    assert!(prompt.contains("apply the session economics policy"));
    assert!(prompt.contains("context_focus-favored signal"));
    assert!(prompt.contains("accept_current_baseline creates an internal checkpoint commit"));
    assert!(prompt.contains("revise_previous restores the previous candidate patch"));
    assert!(!prompt.contains("avoid cumulative context cost"));
    assert!(!prompt.contains("worker_mode=context_focus."));
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
        &SupervisorContextTelemetry::default(),
        DefaultStrategyMode::SupervisedWorker,
    )
    .unwrap();

    assert!(prompt.contains("Artifact index:"));
    assert!(prompt.contains("Inspect the listed core artifact files directly"));
    assert!(prompt.contains("report.md"));
    assert!(prompt.contains("compact worker-turn summary"));
    assert!(prompt.contains("worktree.patch"));
    assert!(prompt.contains("active current repository diff"));
    assert!(prompt.contains("tool-events.jsonl"));
    assert!(prompt.contains("worker tool-call events extracted from structured output"));
    assert!(!prompt.contains("SECRET_ARTIFACT_BODY_SHOULD_NOT_BE_EMBEDDED"));
    assert!(!prompt.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    assert!(!prompt.contains("\"tool\":\"bash\""));
}

#[test]
fn supervisor_spin_out_feedback_prompt_uses_packet_without_artifact_reads() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let report = root.join(REPORT_MD);
    atomic_write(&report, b"SECRET_PACKET_CONTENT").unwrap();
    let packet = json!({
        "kind": "mixmod-spin-out-supervisor-review-packet",
        "artifacts": [{
            "path": "worker/report.md",
            "file_name": REPORT_MD,
            "content": "SECRET_PACKET_CONTENT"
        }]
    });

    let prompt = supervisor_spin_out_feedback_prompt(
        root,
        &[report],
        "decide",
        &WorkerSupervisorGuidance::default(),
        &SupervisorContextTelemetry::default(),
        DefaultStrategyMode::SupervisedWorker,
        &packet,
    )
    .unwrap();

    assert!(prompt.contains("one-shot spin-out supervisor reviewer"));
    assert!(prompt.contains("Use only REVIEW_PACKET"));
    assert!(prompt.contains("Do not run commands"));
    assert!(prompt.contains("inspect files"));
    assert!(prompt.contains("do not fetch more context"));
    assert!(prompt.contains("insufficient_context"));
    assert!(prompt.contains("Existing review policy may say"));
    assert!(prompt.contains("use packet excerpts only"));
    assert!(prompt.contains("SECRET_PACKET_CONTENT"));
    assert!(!prompt.contains("Inspect the listed core artifact files directly"));
    assert!(!prompt.contains("Artifact index:"));
}

#[test]
fn supervisor_spin_out_debug_prompt_keeps_delegation_decision_audit() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let prompt = supervisor_spin_out_feedback_prompt_with_debug_profile_fit(
        root,
        &[],
        "decide",
        &WorkerSupervisorGuidance::default(),
        &SupervisorContextTelemetry::default(),
        DefaultStrategyMode::WorkerBuildSupervisorFix,
        &json!({"kind":"mixmod-spin-out-supervisor-review-packet","artifacts":[]}),
    )
    .unwrap();

    assert!(prompt.contains("Debug delegation-decision audit"));
    assert!(prompt.contains("\"delegation_decision\""));
    assert!(prompt.contains("takeover worker patch"));
}

#[test]
fn supervisor_prompts_include_selected_worker_model_guidance() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let guidance = MixmodConfig::default().worker_supervisor_guidance();
    let feedback_prompt = supervisor_feedback_prompt(
        root,
        &[root.join("missing-report.md")],
        "decide",
        &guidance,
        &SupervisorContextTelemetry::default(),
        DefaultStrategyMode::SupervisedWorker,
    )
    .unwrap();

    assert!(feedback_prompt.contains("Supervisor-only worker-model guidance"));
    assert!(feedback_prompt.contains("Worker shape contract:"));
    assert!(feedback_prompt.contains("Patch-request decomposition contract"));
    assert!(feedback_prompt.contains("one bounded, reviewable implementation slice"));
    assert!(feedback_prompt.contains("Implementation slice policy"));
    assert!(feedback_prompt.contains("implementation surface, not only by end-user behavior"));
    assert!(feedback_prompt.contains("One user-visible behavior can still be too broad"));
    assert!(feedback_prompt.contains("parser/AST, runtime or environment state"));
    assert!(feedback_prompt.contains("When generic task coherence conflicts"));
    assert!(feedback_prompt.contains("hand off the next slice only, not the full task"));
    assert!(feedback_prompt.contains("shape the worker request yourself before emitting JSON"));
    assert!(feedback_prompt.contains("Do not copy the list to the worker"));
    assert!(feedback_prompt.contains("Use relevant bullets as constraints"));
    assert!(feedback_prompt.contains("handoff shape"));
    assert!(feedback_prompt.contains("focused source edits"));
    assert!(feedback_prompt.contains("broad autonomous design work"));
    assert!(feedback_prompt.contains("short worker_turn_shape=patch_request"));
    assert!(feedback_prompt.contains("supervisor tokens cost"));
    assert!(feedback_prompt.contains("directionally useful but messy parser"));
    assert!(feedback_prompt.contains("generated-code"));
    assert!(feedback_prompt.contains("end-to-end integration across slices"));
    assert!(feedback_prompt.contains("task-derived behavior evidence"));
    assert!(feedback_prompt.contains("global environment repair"));

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
    assert!(brief_prompt.contains("Mission: complete the task while minimizing"));
    assert!(brief_prompt.contains("Local worker tokens are cheap"));
    assert!(brief_prompt.contains("Choose the cheapest reliable next worker handoff"));
    assert!(brief_prompt.contains("still include worker_turn_shape and related boundary fields"));
    assert!(brief_prompt.contains("Worker shape contract:"));
    assert!(brief_prompt.contains("Patch-request decomposition contract"));
    assert!(brief_prompt.contains("use worker_turn_shape=\"patch_request\""));
    assert!(brief_prompt.contains("Choose one bounded, reviewable implementation slice"));
    assert!(brief_prompt.contains("do not treat one end-to-end behavior as one slice"));
    assert!(brief_prompt.contains("Do not emit worker_turn_shape=\"bounded_feature_slice\""));
    assert!(brief_prompt.contains("Implementation slice policy"));
    assert!(brief_prompt.contains("implementation surface, not only by end-user behavior"));
    assert!(brief_prompt.contains("One user-visible behavior can still be too broad"));
    assert!(brief_prompt.contains("A request crossing multiple layers is broad"));
    assert!(brief_prompt.contains("source edits are combined with tests/checks"));
    assert!(brief_prompt.contains("obey the profile by shrinking"));
    assert!(brief_prompt.contains("patch-size guidance as a decomposition budget"));
    assert!(brief_prompt.contains("one bounded, reviewable implementation slice"));
    assert!(brief_prompt.contains("hand off the next slice only, not the full task"));
    assert!(brief_prompt.contains("known file boundary"));
    assert!(brief_prompt.contains("reasoning before editing"));
    assert!(brief_prompt.contains("large effective context"));
    assert!(brief_prompt.contains("short worker_turn_shape=patch_request"));
    assert!(brief_prompt.contains("supervisor tokens cost"));
    assert!(brief_prompt.contains("Worker patch-size guidance"));
    assert!(brief_prompt.contains("worker_turn_shape=planning_probe"));
    assert!(brief_prompt.contains("propose the next request"));
    assert!(brief_prompt.contains("one to three focused files"));
    assert!(brief_prompt.contains("expected around 100 changed lines"));
    assert!(brief_prompt.contains("soft maximum around 250 changed lines"));
    assert!(brief_prompt.contains("worker_turn_shape=patch_request"));
    assert!(brief_prompt.contains("smallest reviewable implementation slice"));
    assert!(brief_prompt.contains("files list as a likely read queue"));
    assert!(brief_prompt.contains("do not list large or generated files"));
    assert!(brief_prompt.contains("human-authored source edits"));
    assert!(brief_prompt.contains("generated outputs"));
    assert!(brief_prompt.contains("intentional generated-output step"));
    assert!(brief_prompt.contains("separate authored-source edits"));
    assert!(brief_prompt.contains("after a source diff exists"));
    assert!(brief_prompt.contains("not manual full-file inspection"));
    assert!(brief_prompt.contains("transient generator/debug/build sidecars"));
    assert!(brief_prompt.contains("changed-file lists and patch stats"));
    assert!(brief_prompt.contains("helpers, options, parser/generated code"));
    assert!(brief_prompt.contains("Do not trust compile success"));
    assert!(brief_prompt.contains("task-derived behavior evidence"));
    assert!(brief_prompt.contains("base path plus modifiers"));
    assert!(brief_prompt.contains("one modifier family later"));
    assert!(
        brief_prompt.contains("include a literal anchor only when it prevents worker wandering")
    );
    assert!(brief_prompt.contains("context overflow"));
    assert!(brief_prompt.contains("worker_mode=context_focus"));
    assert!(brief_prompt.contains("repo-level evidence"));
    assert!(brief_prompt.contains("obey the worker shape contract"));
    assert!(brief_prompt.contains("smallest reviewable implementation slice"));
    assert!(brief_prompt.contains("broaden only when worker evidence shows"));
    assert!(brief_prompt.contains("Handoff requirements:"));
    assert!(brief_prompt.contains("exact_edits is optional"));
    assert!(brief_prompt.contains("optional and sparse"));
    assert!(brief_prompt.contains("stop_condition"));
    assert!(brief_prompt.contains("scope_rationale"));
    assert!(!brief_prompt.contains("profile_fit"));
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
    assert!(brief_prompt.contains("acceptable output boundary"));
    assert!(brief_prompt.contains("unrelated generator churn"));
    assert!(brief_prompt.contains("trust that route"));
    assert!(brief_prompt.contains("worker_turn_shape=bounded_feature_slice"));
    assert!(brief_prompt.contains("prefer worker_turn_shape=\"bounded_feature_slice\""));
    assert!(brief_prompt.contains("patch-first"));
    assert!(brief_prompt.contains("toolchain archaeology"));
    assert!(brief_prompt.contains("current accumulated patch"));
    assert!(brief_prompt.contains("end-to-end behavior"));
}

#[test]
fn supervisor_worker_brief_debug_profile_fit_adds_audit_field() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = MixmodConfig::default();
    ModelOverrides::new(
        None,
        Some("openrouter/deepseek/deepseek-v4-flash".to_string()),
    )
    .apply_to_config(&mut config)
    .unwrap();
    let guidance = config.worker_supervisor_guidance();

    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Typed variables",
  "instructions": "Add typed variable declarations.",
  "tests": []
}"#,
    )
    .unwrap();

    let prompt = supervisor_worker_brief_prompt_with_debug_profile_fit(
        root,
        &task,
        &guidance,
        SupervisorInitMode::Compact,
    )
    .unwrap();

    assert!(prompt.contains("Debug profile-fit audit"));
    assert!(prompt.contains(r#"Include "profile_fit" on expected-patch handoffs"#));
    assert!(prompt.contains("file_count, implementation layers"));
    assert!(prompt.contains("generated_or_large_files"));
    assert!(prompt.contains("expected_patch_fit"));
    assert!(prompt.contains("profile_risk"));
    assert!(prompt.contains("scope_adjustment"));
    assert!(prompt.contains("this exact turn_goal, file list, stop_condition, and checks"));
    assert!(prompt.contains("shrink the handoff to the next reviewable slice"));
    assert!(prompt.contains(r#""profile_fit":{"file_count":0"#));
}

#[test]
fn supervisor_prompt_uses_general_patch_request_decomposition_for_minimax() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = MixmodConfig::default();
    ModelOverrides::new(None, Some("openrouter/minimax/minimax-m3".to_string()))
        .apply_to_config(&mut config)
        .unwrap();
    let guidance = config.worker_supervisor_guidance();

    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Typed variables",
  "instructions": "Add typed variable declarations.",
  "tests": []
}"#,
    )
    .unwrap();

    let prompt =
        supervisor_worker_brief_prompt(root, &task, &guidance, SupervisorInitMode::Compact)
            .unwrap();

    assert!(prompt.contains("Patch-request decomposition contract"));
    assert!(prompt.contains(r#"use worker_turn_shape="patch_request""#));
    assert!(prompt.contains("one bounded, reviewable implementation slice"));
    assert!(prompt.contains("implementation surface, not only by end-user behavior"));
    assert!(prompt.contains("multiple independent behaviors"));
    assert!(prompt.contains("hand off the next slice only, not the full task"));
    assert!(prompt.contains("worker-visible stop_condition"));
    assert!(prompt.contains("scope_rationale"));
    assert!(prompt.contains("profile's patch-size guidance"));
    assert!(prompt.contains("known file boundary"));
    assert!(prompt.contains("A first implementation phase should be a real boundary"));
}

#[test]
fn supervisor_prompt_uses_general_patch_request_decomposition_for_deepseek() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = MixmodConfig::default();
    ModelOverrides::new(
        None,
        Some("openrouter/deepseek/deepseek-v4-flash".to_string()),
    )
    .apply_to_config(&mut config)
    .unwrap();
    let guidance = config.worker_supervisor_guidance();

    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Typed variables",
  "instructions": "Add typed variable declarations.",
  "tests": []
}"#,
    )
    .unwrap();

    let prompt =
        supervisor_worker_brief_prompt(root, &task, &guidance, SupervisorInitMode::Compact)
            .unwrap();

    assert!(prompt.contains("Patch-request decomposition contract"));
    assert!(prompt.contains(r#"use worker_turn_shape="patch_request""#));
    assert!(prompt.contains("one bounded, reviewable implementation slice"));
    assert!(prompt.contains("do not treat one end-to-end behavior as one slice"));
    assert!(prompt.contains("independent behaviors, parser/AST"));
    assert!(prompt.contains("verification steps"));
    assert!(prompt.contains("likely exceeds the worker patch budget"));
    assert!(prompt.contains("decompose it yourself before emitting JSON"));
    assert!(prompt.contains("hand off the next slice only, not the full task"));
    assert!(prompt.contains("name the command or boundary"));
    assert!(prompt.contains("stop after the slice has one useful tracked diff"));
    assert!(prompt.contains("profile's patch-size guidance"));
    assert!(prompt.contains("within the selected worker shape contract"));
    assert!(prompt.contains("selected worker profile explicitly supports that scope"));
    assert!(prompt.contains("patch-size guidance as a decomposition budget"));
    assert!(prompt.contains("Do not ask for full-task or multi-phase scope on the first turn"));
    assert!(prompt.contains("After a successful bounded patch_request"));
    assert!(prompt.contains("prior worker evidence shows DeepSeek handled it cleanly"));
    assert!(
        !prompt.contains("DeepSeek V4 Flash first expected-patch implementation handoff contract")
    );
    assert!(!prompt.contains("generated parser output"));
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
    for name in RUN_CORE_REVIEW_ARTIFACTS
        .iter()
        .chain(RUN_DIAGNOSTIC_ARTIFACTS.iter())
    {
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
    assert!(paths.contains(&REVIEW_SIGNALS_JSON.to_string()));
    assert!(paths.contains(&CHANGES_PATCH.to_string()));
    assert!(!paths.contains(&WORKTREE_PATCH.to_string()));
    assert!(!paths.contains(&TOOL_EVENTS_JSONL.to_string()));
    assert!(!paths.contains(&METRICS_JSON.to_string()));
}
