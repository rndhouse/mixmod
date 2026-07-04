use super::*;

#[test]
fn run_writes_full_artifact_bundle() {
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
        run_mixmod_task(root, DelegationMode::Patch, &task, &run_dir, &FakeRunner).unwrap();

    assert_eq!(receipt.status, "success");
    for artifact in [
        "receipt.json",
        "task.json",
        "report.md",
        "session.jsonl",
        "worktree.patch",
        "changes.patch",
        "interventions.jsonl",
        "metrics.json",
        "logs/opencode.stdout.txt",
        "logs/opencode.stderr.txt",
    ] {
        assert!(run_dir.join(artifact).exists());
    }
    assert!(receipt.interventions.ends_with("interventions.jsonl"));
    let patch = fs::read_to_string(run_dir.join("changes.patch")).unwrap();
    assert!(patch.contains("src/generated.rs"));
    let interventions = fs::read_to_string(run_dir.join("interventions.jsonl")).unwrap();
    assert_eq!(interventions.lines().count(), 1);
    let handoff: Value = serde_json::from_str(interventions.lines().next().unwrap()).unwrap();
    assert_eq!(get_str(&handoff, "kind"), Some("worker_handoff"));
    assert_eq!(get_str(&handoff, "outcome"), Some("instruction_written"));
    assert!(!root.join(".mixmod").exists());
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
    let receipt = run_mixmod_task(root, DelegationMode::Patch, &task, &run_dir, &runner).unwrap();

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
    let receipt = run_mixmod_task_with_session(
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
                model: Some(DEFAULT_OPENCODE_OLLAMA_MODEL.to_string()),
                model_arg: Some(format!("fake-local/{DEFAULT_OPENCODE_OLLAMA_MODEL}")),
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
        run_mixmod_task(root, DelegationMode::Patch, &task, &run_dir, &NoEditRunner).unwrap();

    assert_eq!(receipt.status, "success");
    let metrics = read_json_file(&run_dir.join("metrics.json")).unwrap();
    assert_eq!(get_bool(&metrics, "expect_patch"), Some(false));
    assert_eq!(
        get_bool(&metrics, "empty_patch_followup_triggered"),
        Some(false)
    );
}
