use super::*;

#[test]
fn worker_turn_writes_full_artifact_bundle() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git(root);
    fs::write(root.join("README.md"), "base\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(root)
        .output()
        .unwrap();

    let task = root.join("example.task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Generate file",
  "instructions": "Create a small generated file.",
  "files": ["src/generated.rs"],
  "tests": ["echo ok"]
}"#,
    )
    .unwrap();

    let run_dir = state_layout(root).runs().join("example");
    let receipt =
        run_worker_turn(root, DelegationMode::Patch, &task, &run_dir, &FakeRunner).unwrap();

    assert_eq!(receipt.status, "success");
    for artifact in [
        "receipt.json",
        "task.json",
        "report.md",
        "review-signals.json",
        "session.jsonl",
        "reasoning-trace.jsonl",
        "tool-events.jsonl",
        "worktree.patch",
        "changes.patch",
        "interventions.jsonl",
        "metrics.json",
        "logs/opencode.events.jsonl",
        "logs/opencode.stdout.txt",
        "logs/opencode.stderr.txt",
    ] {
        assert!(run_dir.join(artifact).exists());
    }
    assert!(receipt.interventions.ends_with("interventions.jsonl"));
    let patch = fs::read_to_string(run_dir.join("changes.patch")).unwrap();
    assert!(patch.contains("src/generated.rs"));
    let review_signals = read_json_file(&run_dir.join("review-signals.json")).unwrap();
    assert_eq!(get_str(&review_signals, "status"), Some("success"));
    assert_eq!(get_u64(&review_signals, "changed_file_count"), Some(1));
    assert_eq!(get_u64(&review_signals, "tool_event_count"), Some(0));
    let interventions = fs::read_to_string(run_dir.join("interventions.jsonl")).unwrap();
    assert_eq!(interventions.lines().count(), 1);
    let handoff: Value = serde_json::from_str(interventions.lines().next().unwrap()).unwrap();
    assert_eq!(get_str(&handoff, "kind"), Some("worker_handoff"));
    assert_eq!(get_str(&handoff, "outcome"), Some("instruction_written"));
    assert!(!root.join(".mixmod").exists());
}

#[test]
fn codex_worker_segments_count_as_worker_tokens() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git(root);
    fs::write(root.join("README.md"), "base\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(root)
        .output()
        .unwrap();

    let task = root.join("example.task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Verify current state",
  "instructions": "Report current state.",
  "expect_patch": false,
  "files": [],
  "tests": []
}"#,
    )
    .unwrap();

    struct CodexTokenRunner;
    impl AgentHarness for CodexTokenRunner {
        fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
            let mut output = minimal_opencode_output();
            output.backend = AgentBackend::Codex;
            output.command_for_metrics = vec!["codex".to_string(), "app-server".to_string()];
            output.segments = vec![json!({
                "backend": "codex",
                "input_tokens": 120,
                "cached_input_tokens": 80,
                "output_tokens": 9,
                "reasoning_tokens": 3,
                "total_tokens": 129
            })];
            output.exit_status = Some(0);
            output.success = true;
            output.stdout = b"Summary: current state verified.\n".to_vec();
            output.provider = Some("codex".to_string());
            output.model = Some("gpt-5.5".to_string());
            output.model_arg = Some("gpt-5.5:high".to_string());
            output.session_label = Some(request.session_id.clone());
            output.session_id = Some("codex-thread".to_string());
            output.backend_activity_observed = true;
            Ok(output)
        }
    }

    let run_dir = state_layout(root).runs().join("codex-worker");
    let receipt = run_worker_turn(
        root,
        DelegationMode::Patch,
        &task,
        &run_dir,
        &CodexTokenRunner,
    )
    .unwrap();

    assert_eq!(receipt.status, "success");
    let metrics = read_json_file(&run_dir.join("metrics.json")).unwrap();
    assert_eq!(get_str(&metrics, "worker_backend"), Some("codex"));
    assert_eq!(get_u64(&metrics, "worker_input_tokens"), Some(120));
    assert_eq!(get_u64(&metrics, "worker_cached_input_tokens"), Some(80));
    assert_eq!(get_u64(&metrics, "worker_output_tokens"), Some(9));
    assert_eq!(get_u64(&metrics, "worker_reasoning_tokens"), Some(3));
    assert_eq!(get_u64(&metrics, "worker_total_tokens"), Some(129));
    assert_eq!(get_u64(&metrics, "codex_token_usage"), Some(129));
    assert_eq!(
        get_str(&metrics, "worker_token_usage_source"),
        Some("codex_worker_segment_tokens")
    );
}

#[test]
fn empty_patch_followup_runs_once_when_patch_expected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git(root);
    fs::write(root.join("README.md"), "base\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(root)
        .output()
        .unwrap();

    let task = root.join("example.task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Generate file",
  "instructions": "Create a small generated file.",
  "expect_patch": true,
  "files": ["src/generated.rs"],
  "tests": []
}"#,
    )
    .unwrap();
    let runner = EmptyPatchThenPatchRunner::new();

    let run_dir = state_layout(root).runs().join("example");
    let receipt = run_worker_turn(root, DelegationMode::Patch, &task, &run_dir, &runner).unwrap();

    assert_eq!(receipt.status, "success");
    assert_eq!(runner.calls.load(AtomicOrdering::SeqCst), 2);
    let patch = fs::read_to_string(run_dir.join("changes.patch")).unwrap();
    assert!(patch.contains("src/generated.rs"));
    assert!(run_dir.join("empty-patch-followup/task.json").exists());
    assert!(
        run_dir
            .join("empty-patch-followup/opencode-instructions.md")
            .exists()
    );
    let followup_instruction =
        fs::read_to_string(run_dir.join("empty-patch-followup/opencode-instructions.md")).unwrap();
    assert!(followup_instruction.contains("Mixmod-managed state lives outside this repository"));
    assert!(!followup_instruction.contains("Task file:"));
    assert!(!followup_instruction.contains("Artifact directory:"));
    let metrics = read_json_file(&run_dir.join("metrics.json")).unwrap();
    assert_eq!(get_bool(&metrics, "expect_patch"), Some(true));
    assert_eq!(
        get_bool(&metrics, "empty_patch_followup_triggered"),
        Some(true)
    );
    assert_eq!(
        get_bool(&metrics, "empty_patch_followup_performed"),
        Some(true)
    );
    assert_eq!(
        get_bool(&metrics, "empty_patch_followup_patch_created"),
        Some(true)
    );
    assert_eq!(get_u64(&metrics, "intervention_count"), Some(2));
    assert_eq!(
        get_string_array(&metrics, "intervention_kinds"),
        vec!["worker_handoff", "empty_patch_followup"]
    );
    let interventions = fs::read_to_string(run_dir.join("interventions.jsonl")).unwrap();
    assert!(interventions.contains("\"kind\":\"empty_patch_followup\""));
    assert!(interventions.contains("\"outcome\":\"patch_created\""));
}

#[test]
fn recovery_can_be_disabled_for_first_worker_inspection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git(root);
    fs::write(root.join("README.md"), "base\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "pre-existing user edit\n").unwrap();

    let task = root.join("example.task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Generate file",
  "instructions": "Create a small generated file.",
  "expect_patch": true,
  "files": ["README.md", "src/generated.rs"],
  "tests": []
}"#,
    )
    .unwrap();
    let runner = EmptyPatchThenPatchRunner::new();

    let run_dir = state_layout(root).runs().join("example");
    let receipt = run_worker_turn_with_options(
        root,
        DelegationMode::Patch,
        &task,
        &run_dir,
        &runner,
        false,
        WorkerTurnOptions {
            resume_session_id: None,
            allow_auto_followups: false,
            ..WorkerTurnOptions::default()
        },
    )
    .unwrap();

    assert_eq!(receipt.status, "needs_supervisor");
    assert_eq!(runner.calls.load(AtomicOrdering::SeqCst), 1);
    assert!(!run_dir.join("empty-patch-followup").exists());
    let patch = fs::read_to_string(run_dir.join("changes.patch")).unwrap();
    assert!(patch.trim().is_empty());
    let metrics = read_json_file(&run_dir.join("metrics.json")).unwrap();
    assert_eq!(
        get_bool(&metrics, "empty_patch_followup_triggered"),
        Some(false)
    );
    assert_eq!(get_u64(&metrics, "intervention_count"), Some(1));
}

#[test]
fn worker_self_review_is_optional_and_reuses_worker_session() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git(root);
    fs::write(root.join("README.md"), "base\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(root)
        .output()
        .unwrap();

    let task = root.join("example.task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Generate file",
  "instructions": "Create a small generated file.",
  "expect_patch": true,
  "files": ["src/generated.rs"],
  "tests": []
}"#,
    )
    .unwrap();
    let runner = PatchThenSelfReviewRunner::new();
    let run_dir = state_layout(root).runs().join("self-review");
    let receipt = run_worker_turn_with_options(
        root,
        DelegationMode::Patch,
        &task,
        &run_dir,
        &runner,
        false,
        WorkerTurnOptions {
            worker_self_review: true,
            ..WorkerTurnOptions::default()
        },
    )
    .unwrap();

    assert_eq!(receipt.status, "success");
    assert_eq!(runner.calls.load(AtomicOrdering::SeqCst), 2);
    assert!(run_dir.join("worker-self-review/task.json").exists());
    assert!(
        run_dir
            .join("worker-self-review/opencode-instructions.md")
            .exists()
    );
    let patch = fs::read_to_string(run_dir.join("changes.patch")).unwrap();
    assert!(patch.contains("src/generated.rs"));
    assert!(!patch.contains("temporary debug marker"));
    assert!(!patch.contains("README.md"));
    let review_patch =
        fs::read_to_string(run_dir.join("worker-self-review/worker-session.patch")).unwrap();
    assert!(review_patch.contains("src/generated.rs"));
    assert!(!review_patch.contains("README.md"));
    let metrics = read_json_file(&run_dir.join("metrics.json")).unwrap();
    assert_eq!(get_bool(&metrics, "worker_self_review_enabled"), Some(true));
    assert_eq!(
        get_bool(&metrics, "worker_self_review_triggered"),
        Some(true)
    );
    assert_eq!(
        get_bool(&metrics, "worker_self_review_performed"),
        Some(true)
    );
    assert_eq!(
        get_bool(&metrics, "worker_self_review_patch_changed"),
        Some(true)
    );
    assert_eq!(get_u64(&metrics, "intervention_count"), Some(2));
    assert_eq!(
        get_string_array(&metrics, "intervention_kinds"),
        vec!["worker_handoff", "worker_self_review"]
    );
    let interventions = fs::read_to_string(run_dir.join("interventions.jsonl")).unwrap();
    assert!(interventions.contains("\"kind\":\"worker_self_review\""));
    assert!(interventions.contains("\"session_policy\":\"same_session\""));
}

#[test]
fn revision_noop_followup_reuses_worker_session_and_requires_delta() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git(root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("README.md"), "base\n").unwrap();
    fs::write(
        root.join("src/existing.rs"),
        "pub fn existing() -> &'static str {\n    \"base\"\n}\n",
    )
    .unwrap();
    Command::new("git")
        .args(["add", "README.md", "src/existing.rs"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(root)
        .output()
        .unwrap();
    fs::write(
        root.join("src/existing.rs"),
        "pub fn existing() -> &'static str {\n    \"candidate\"\n}\n",
    )
    .unwrap();

    let task = root.join("revision.task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Revision task",
  "instructions": "Original task.\n\nMessage to worker: Apply the exact requested revision now.",
  "expect_patch": true,
  "files": ["src/existing.rs", "src/revised.rs"],
  "tests": [],
  "acceptance": ["cargo test"],
  "context": {
    "expect_patch": true,
    "revision": {
      "delta_expected": true,
      "message_to_worker": "Apply the exact requested revision now.",
      "worker_mode": "continue",
      "patch_decision": "revise_current",
      "focus_files": ["src/revised.rs"],
      "required_checks": ["cargo test"]
    }
  }
}"#,
    )
    .unwrap();
    let runner = RevisionNoopThenPatchRunner::new();
    let run_dir = state_layout(root).runs().join("revision");
    let receipt = run_worker_turn_with_session(
        root,
        DelegationMode::Patch,
        &task,
        &run_dir,
        &runner,
        false,
        Some("ses_revision".to_string()),
    )
    .unwrap();

    assert_eq!(receipt.status, "success");
    assert_eq!(runner.calls.load(AtomicOrdering::SeqCst), 2);
    assert!(run_dir.join("revision-noop-followup/task.json").exists());
    assert!(
        run_dir
            .join("revision-noop-followup/opencode-instructions.md")
            .exists()
    );
    assert!(!run_dir.join("empty-patch-followup").exists());
    let patch = fs::read_to_string(run_dir.join("changes.patch")).unwrap();
    assert!(patch.contains("src/revised.rs"));
    assert!(!patch.contains("src/existing.rs"));
    let metrics = read_json_file(&run_dir.join("metrics.json")).unwrap();
    assert_eq!(get_bool(&metrics, "revision_delta_expected"), Some(true));
    assert_eq!(
        get_bool(&metrics, "revision_noop_followup_triggered"),
        Some(true)
    );
    assert_eq!(
        get_bool(&metrics, "revision_noop_followup_performed"),
        Some(true)
    );
    assert_eq!(
        get_bool(&metrics, "revision_noop_followup_patch_created"),
        Some(true)
    );
    assert_eq!(
        get_bool(&metrics, "empty_patch_followup_triggered"),
        Some(false)
    );
    assert!(get_u64(&metrics, "revision_delta_bytes").unwrap() > 0);
    assert_eq!(get_u64(&metrics, "intervention_count"), Some(2));
    assert_eq!(
        get_string_array(&metrics, "intervention_kinds"),
        vec!["worker_handoff", "revision_noop_followup"]
    );
    let interventions = fs::read_to_string(run_dir.join("interventions.jsonl")).unwrap();
    assert!(interventions.contains("\"kind\":\"revision_noop_followup\""));
    assert!(interventions.contains("\"session_policy\":\"same_session\""));
}

#[test]
fn empty_patch_is_allowed_when_patch_not_expected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git(root);
    fs::write(root.join("README.md"), "base\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(root)
        .output()
        .unwrap();

    let task = root.join("example.task.json");
    atomic_write(
        &task,
        br#"{
  "title": "No edit needed",
  "instructions": "Explain why no repository edit is needed.",
  "expect_patch": false,
  "files": [],
  "tests": []
}"#,
    )
    .unwrap();
    struct NoEditRunner;
    impl AgentHarness for NoEditRunner {
        fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
            Ok(AgentOutput {
                backend: AgentBackend::OpenCode,
                command_for_metrics: vec!["fake-opencode".to_string()],
                segments: Vec::new(),
                exit_status: Some(0),
                success: true,
                stdout: b"Summary: no patch is needed.\n".to_vec(),
                stderr: Vec::new(),
                provider: Some("fake-local".to_string()),
                model: Some(DEFAULT_OPENCODE_LOCAL_MODEL.to_string()),
                model_arg: Some(format!("fake-local/{DEFAULT_OPENCODE_LOCAL_MODEL}")),
                session_label: Some(request.session_id.clone()),
                session_id: Some("ses_no_edit".to_string()),
                resume_session_id: request.resume_session_id.clone(),
                session_reused: request.resume_session_id.is_some(),
                interrupted_by_supervisor: false,
                supervisor_control_action: None,
                supervisor_control_events: Vec::new(),
                timed_out: false,
                idle_timed_out: false,
                heartbeat_count: 0,
                require_local: false,
                local_inference_verified: false,
                gpu_activity_observed: false,
                backend_activity_observed: false,
                verification_notes: Vec::new(),
            })
        }
    }

    let run_dir = state_layout(root).runs().join("example");
    let receipt =
        run_worker_turn(root, DelegationMode::Patch, &task, &run_dir, &NoEditRunner).unwrap();

    assert_eq!(receipt.status, "success");
    let metrics = read_json_file(&run_dir.join("metrics.json")).unwrap();
    assert_eq!(get_bool(&metrics, "expect_patch"), Some(false));
    assert_eq!(
        get_bool(&metrics, "empty_patch_followup_triggered"),
        Some(false)
    );
}
