use super::*;
use clap::Parser;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::Duration;
use tempfile::TempDir;

struct FakeRunner;

impl OpenCodeRunner for FakeRunner {
    fn run(&self, request: &OpenCodeRequest) -> Result<OpenCodeOutput> {
        fs::create_dir_all(request.root.join("src")).with_context(|| {
            format!(
                "failed to create fake source dir under {}",
                request.root.display()
            )
        })?;
        atomic_write(
            &request.root.join("src/generated.rs"),
            b"pub fn generated() -> &'static str {\n    \"ok\"\n}\n",
        )?;
        Ok(OpenCodeOutput {
            command_for_metrics: vec!["fake-opencode".to_string()],
            opencode_segments: Vec::new(),
            exit_status: Some(0),
            success: true,
            stdout: b"Summary: generated a file\nTests: not run\n".to_vec(),
            stderr: Vec::new(),
            provider: Some("fake-local".to_string()),
            model: Some(DEFAULT_OPENCODE_OLLAMA_MODEL.to_string()),
            model_arg: Some(format!("fake-local/{DEFAULT_OPENCODE_OLLAMA_MODEL}")),
            session_label: Some(request.session_id.clone()),
            session_id: Some(request.session_id.clone()),
            resume_session_id: request.resume_session_id.clone(),
            worker_session_reused: request.resume_session_id.is_some(),
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

struct EmptyPatchThenPatchRunner {
    calls: AtomicUsize,
}

impl EmptyPatchThenPatchRunner {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl OpenCodeRunner for EmptyPatchThenPatchRunner {
    fn run(&self, request: &OpenCodeRequest) -> Result<OpenCodeOutput> {
        let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        let (stdout, resume_session_id) = if call == 0 {
            assert!(request.resume_session_id.is_none());
            (
                b"Summary: I found the edit but did not modify files.\n".to_vec(),
                None,
            )
        } else {
            assert_eq!(
                request.resume_session_id.as_deref(),
                Some("ses_empty_patch")
            );
            assert!(request.instruction.contains("Empty-Patch Follow-Up"));
            fs::create_dir_all(request.root.join("src"))?;
            atomic_write(
                &request.root.join("src/generated.rs"),
                b"pub fn generated() -> &'static str {\n    \"followup\"\n}\n",
            )?;
            (
                b"Summary: made the intended edit after empty-patch follow-up.\n".to_vec(),
                Some("ses_empty_patch".to_string()),
            )
        };
        Ok(OpenCodeOutput {
            command_for_metrics: vec!["fake-opencode".to_string()],
            opencode_segments: vec![json!({"call": call})],
            exit_status: Some(0),
            success: true,
            stdout,
            stderr: Vec::new(),
            provider: Some("fake-local".to_string()),
            model: Some(DEFAULT_OPENCODE_OLLAMA_MODEL.to_string()),
            model_arg: Some(format!("fake-local/{DEFAULT_OPENCODE_OLLAMA_MODEL}")),
            session_label: Some(request.session_id.clone()),
            session_id: Some("ses_empty_patch".to_string()),
            resume_session_id,
            worker_session_reused: request.resume_session_id.is_some(),
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

fn init_git(root: &Path) {
    Command::new("git")
        .arg("init")
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(root)
        .output()
        .unwrap();
}

fn minimal_opencode_output() -> OpenCodeOutput {
    OpenCodeOutput {
        command_for_metrics: Vec::new(),
        opencode_segments: Vec::new(),
        exit_status: None,
        success: false,
        stdout: Vec::new(),
        stderr: Vec::new(),
        provider: None,
        model: None,
        model_arg: None,
        session_label: None,
        session_id: None,
        resume_session_id: None,
        worker_session_reused: false,
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
    }
}

#[test]
fn exit_status_label_names_supervisor_interrupt() {
    let mut output = minimal_opencode_output();
    output.interrupted_by_supervisor = true;

    assert_eq!(
        opencode_exit_status_label(&output),
        "interrupted-by-supervisor"
    );
}

#[test]
fn summary_reports_captured_patch_when_tests_fail() {
    let output = minimal_opencode_output();
    let stats = PatchStats {
        files: vec!["sympy/core/power.py".to_string()],
        changed_line_count: 2,
        added_lines: 2,
        removed_lines: 0,
    };

    let summary = build_run_summary(
        "needs_supervisor",
        DelegationMode::Patch,
        &output,
        &stats,
        "failed",
    );

    assert!(summary.contains("with 1 file(s) and 2 line(s) changed"));
    assert!(!summary.contains("no patch was captured"));
}

#[test]
fn supervise_args_launch_background_run_with_resume() {
    let args = supervise_run_args(
        DelegationMode::Patch,
        Path::new("/tmp/task.json"),
        Path::new("/tmp/run"),
        true,
        Some("ses_123"),
        &ModelOverrides::new(
            Some("gpt-5.5:high".to_string()),
            Some("mixmod-local-ollama/qwen3.6:27b".to_string()),
        ),
    );

    assert_eq!(args[0], "run-worker");
    assert_eq!(args[1], "patch");
    assert!(args.contains(&"--require-local".to_string()));
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--resume-session" && pair[1] == "ses_123")
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--supervisor-model" && pair[1] == "gpt-5.5:high")
    );
    assert!(args.windows(2).any(|pair| pair[0] == "--worker-model"
        && pair[1] == "mixmod-local-ollama/qwen3.6:27b"));
}

#[test]
fn debug_commands_are_gated_by_default() {
    let error = ensure_debug_command_enabled("mixmod run-worker").unwrap_err();

    assert!(error.to_string().contains("mixmod exec"));
    assert!(error.to_string().contains("MIXMOD_DEBUG_COMMANDS=1"));
}

#[test]
fn exec_command_is_public_cli_surface() {
    let cli = Cli::try_parse_from([
        "mixmod",
        "exec",
        "--task",
        "task.json",
        "--out",
        ".mixmod/runs/demo",
    ])
    .unwrap();

    match cli.command {
        Commands::Exec {
            task,
            out,
            resume_session,
            supervisor_model,
            worker_model,
        } => {
            assert_eq!(task, PathBuf::from("task.json"));
            assert_eq!(out, PathBuf::from(".mixmod/runs/demo"));
            assert!(resume_session.is_none());
            assert!(supervisor_model.is_none());
            assert!(worker_model.is_none());
        }
        _ => panic!("expected exec command"),
    }

    assert!(Cli::try_parse_from(["mixmod", "delegate"]).is_err());
    assert!(
        Cli::try_parse_from([
            "mixmod",
            "exec",
            "--task",
            "task.json",
            "--out",
            ".mixmod/runs/demo",
            "--require-local",
        ])
        .is_err()
    );
}

#[test]
fn exec_accepts_supervisor_and_worker_model_flags() {
    let cli = Cli::try_parse_from([
        "mixmod",
        "exec",
        "--task",
        "task.json",
        "--out",
        ".mixmod/runs/demo",
        "--supervisor-model",
        "gpt-5.5:high",
        "--worker-model",
        "mixmod-local-ollama/qwen3.6:27b",
    ])
    .unwrap();

    match cli.command {
        Commands::Exec {
            supervisor_model,
            worker_model,
            ..
        } => {
            assert_eq!(supervisor_model, Some("gpt-5.5:high".to_string()));
            assert_eq!(
                worker_model,
                Some("mixmod-local-ollama/qwen3.6:27b".to_string())
            );
        }
        _ => panic!("expected exec command"),
    }
}

#[test]
fn model_overrides_apply_supervisor_and_worker_models() {
    let mut config = MixmodConfig::default();

    ModelOverrides::new(
        Some("gpt-5.5:xhigh".to_string()),
        Some("ollama/qwen3.6:27b".to_string()),
    )
    .apply_to_config(&mut config)
    .unwrap();

    assert_eq!(config.frontier.model, "gpt-5.5");
    assert_eq!(config.frontier.reasoning_effort, "xhigh");
    assert_eq!(config.opencode.provider, "ollama");
    assert_eq!(config.opencode.model, "qwen3.6:27b");
    assert!(
        config
            .opencode
            .model_aliases
            .get("qwen3.6:27b")
            .unwrap()
            .contains(&"ollama/qwen3.6:27b".to_string())
    );
}

#[test]
fn model_overrides_reject_invalid_supervisor_effort() {
    let mut config = MixmodConfig::default();
    let error = ModelOverrides::new(Some("gpt-5.5:turbo".to_string()), None)
        .apply_to_config(&mut config)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unsupported supervisor reasoning effort")
    );
}

#[test]
fn codex_supervision_turns_are_read_only_and_non_interactive() {
    let args = codex_exec_turn_args(
        "gpt-5.5",
        "high",
        Path::new("/tmp/work"),
        Path::new("/tmp/work/.mixmod/out/last-message.json"),
        CodexSandbox::ReadOnly,
    );

    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--sandbox" && pair[1] == "read-only")
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == "approval_policy=\"never\"")
    );
    assert!(!args.contains(&"--dangerously-bypass-approvals-and-sandbox".to_string()));
}

#[test]
fn codex_only_baseline_can_write_workspace_files() {
    let args = codex_exec_turn_args(
        "gpt-5.5",
        "high",
        Path::new("/tmp/work"),
        Path::new("/tmp/work/.mixmod/out/last-message.json"),
        CodexSandbox::WorkspaceWrite,
    );

    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--sandbox" && pair[1] == "workspace-write")
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == "approval_policy=\"never\"")
    );
}

#[test]
fn patch_stats_counts_files_and_lines() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n-old\n+new\n+line\n";
    let stats = patch_stats(patch);
    assert_eq!(stats.files, vec!["src/lib.rs"]);
    assert_eq!(stats.added_lines, 2);
    assert_eq!(stats.removed_lines, 1);
    assert_eq!(stats.changed_line_count, 3);
}

#[test]
fn resumed_opencode_args_use_specific_session_without_title() {
    let args = vec![
        "run".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--model".to_string(),
        "local-ollama/qwen3.6:27b".to_string(),
        "--title".to_string(),
        "opencode-session-123".to_string(),
        "Do the task".to_string(),
    ];

    let prepared = prepare_opencode_args(args, Some("ses_abc123"));

    assert_eq!(
        prepared,
        vec![
            "run",
            "--session",
            "ses_abc123",
            "--dangerously-skip-permissions",
            "--model",
            "local-ollama/qwen3.6:27b",
            "Do the task",
        ]
    );
}

fn test_opencode_request(root: &Path) -> OpenCodeRequest {
    OpenCodeRequest {
        root: root.to_path_buf(),
        mode: DelegationMode::Patch,
        task_path: root.join("task.json"),
        out_dir: root.join(".mixmod/runs/test"),
        instruction_path: root.join(".mixmod/runs/test/opencode-instructions.md"),
        instruction: "FULL ORIGINAL INSTRUCTION".to_string(),
        session_id: "opencode-session-test".to_string(),
        resume_session_id: None,
        require_local: false,
    }
}

#[test]
fn interrupt_continue_args_resume_session_with_only_control_message() {
    let temp = TempDir::new().unwrap();
    let request = test_opencode_request(temp.path());
    let args = vec![
        "run".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--model".to_string(),
        "local-ollama/qwen3.6:27b".to_string(),
        "--title".to_string(),
        "opencode-session-test".to_string(),
        "FULL ORIGINAL INSTRUCTION".to_string(),
    ];

    let prepared = prepare_opencode_control_args(
        &args,
        &request,
        Some("ses_existing"),
        "opencode-session-test",
        "Focus and finish.",
    );

    assert_eq!(
        prepared,
        vec![
            "run",
            "--session",
            "ses_existing",
            "--dangerously-skip-permissions",
            "--model",
            "local-ollama/qwen3.6:27b",
            "Focus and finish.",
        ]
    );
}

#[test]
fn context_focus_args_start_fresh_titled_session_with_control_message() {
    let temp = TempDir::new().unwrap();
    let request = test_opencode_request(temp.path());
    let args = vec![
        "run".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--model".to_string(),
        "local-ollama/qwen3.6:27b".to_string(),
        "--session".to_string(),
        "ses_old".to_string(),
        "FULL ORIGINAL INSTRUCTION".to_string(),
    ];

    let prepared = prepare_opencode_control_args(
        &args,
        &request,
        None,
        "opencode-session-test",
        "Fresh focus.",
    );

    assert_eq!(
        prepared,
        vec![
            "run",
            "--title",
            "opencode-session-test",
            "--dangerously-skip-permissions",
            "--model",
            "local-ollama/qwen3.6:27b",
            "Fresh focus.",
        ]
    );
}

#[test]
fn interrupt_continue_restarts_opencode_with_same_session_inside_one_run() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let fake_opencode = root.join("fake-opencode.sh");
    let calls_path = root.join("calls.txt");
    let script = format!(
        r#"#!/bin/sh
printf 'cmd=%s env=%s args=%s\n' "$1" "$OPENCODE_CONFIG" "$*" >> "{}"
if [ "$1" = "models" ]; then
  echo "local-ollama/qwen3.6:27b"
  exit 0
fi
if [ "$1" = "db" ]; then
  echo '[{{"id":"ses_fake"}}]'
  exit 0
fi
case " $* " in
  *" --session ses_fake "*)
    echo "resumed"
    exit 0
    ;;
  *)
    echo "initial"
    exec sleep 30
    ;;
esac
"#,
        calls_path.display()
    );
    atomic_write(&fake_opencode, script.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fake_opencode).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake_opencode, perms).unwrap();
    }

    let out_dir = root.join(".mixmod/runs/control-send-test");
    let request = OpenCodeRequest {
        root: root.to_path_buf(),
        mode: DelegationMode::Patch,
        task_path: root.join("task.json"),
        out_dir: out_dir.clone(),
        instruction_path: out_dir.join("opencode-instructions.md"),
        instruction: "FULL ORIGINAL INSTRUCTION".to_string(),
        session_id: "opencode-session-test".to_string(),
        resume_session_id: None,
        require_local: false,
    };
    let mut config = OpenCodeConfig::default();
    config.local_verification.enabled = false;
    config.heartbeat_seconds = 1;
    config.worker_timeout_seconds = 10;
    config.idle_timeout_seconds = 0;
    let selection = OpenCodeModelSelection {
        provider: "local-ollama".to_string(),
        model: "qwen3.6:27b".to_string(),
        model_arg: "local-ollama/qwen3.6:27b".to_string(),
        require_local: false,
    };
    let control_path = out_dir.join(SUPERVISOR_CONTROL_FILE);
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        atomic_write(
            &control_path,
            serde_json::to_vec_pretty(&json!({
                "action": "interrupt_continue",
                "message_to_worker": "Finish from the current session."
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();
    });

    let output = run_with_local_verification(
        fake_opencode.to_str().unwrap(),
        &[
            "run".to_string(),
            "--title".to_string(),
            "opencode-session-test".to_string(),
            "FULL ORIGINAL INSTRUCTION".to_string(),
        ],
        &request,
        &config,
        &selection,
    )
    .unwrap();
    writer.join().unwrap();

    assert_eq!(output.exit_status, Some(0));
    assert!(!output.interrupted_by_supervisor);
    assert_eq!(output.supervisor_control_events.len(), 1);
    assert_eq!(output.opencode_segments.len(), 2);
    let calls = fs::read_to_string(calls_path).unwrap();
    assert!(
        calls.contains(
            opencode_config_path(root)
                .to_str()
                .expect("OpenCode config path should be UTF-8")
        )
    );
    assert!(calls.contains("--session ses_fake"));
    assert!(calls.contains("Finish from the current session."));
    assert!(!calls.contains("--session ses_fake FULL ORIGINAL INSTRUCTION"));
}

#[test]
fn frontier_feedback_prompt_explains_worker_session_modes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let prompt =
        frontier_feedback_prompt(root, &[root.join("missing-report.md")], "decide").unwrap();

    assert!(prompt.contains("worker_mode=continue to keep the same OpenCode session"));
    assert!(prompt.contains("worker_mode=context_focus to start a new OpenCode session"));
    assert!(prompt.contains("previous worker context is discarded"));
    assert!(prompt.contains("Do not implement code. Do not edit files."));
    assert!(prompt.contains("Do not ask the user for approval."));
    assert!(
        prompt.contains(
            "Prefer revise after failed, empty, distracted, or incomplete worker attempts"
        )
    );
    assert!(prompt.contains("Stop does not permit direct Codex editing."));
}

#[test]
fn supervisor_control_metrics_become_revision_decision() {
    let temp = TempDir::new().unwrap();
    let run_dir = temp.path().join("run");
    fs::create_dir_all(&run_dir).unwrap();
    atomic_write(
        &run_dir.join("metrics.json"),
        serde_json::to_vec_pretty(&json!({
            "supervisor_control_events": [{
                "action": "interrupt_context_focus",
                "worker_mode": "context_focus",
                "message_to_worker": "Ignore setup and edit sympy/core/numbers.py first.",
                "focus_files": ["sympy/core/numbers.py"],
                "required_checks": ["python -m pytest sympy/core/tests/test_power.py -q"],
                "risk": "worker was distracted by dependency setup"
            }]
        }))
        .unwrap()
        .as_slice(),
    )
    .unwrap();

    let decision = supervisor_control_decision_from_metrics(&run_dir)
        .unwrap()
        .unwrap();

    assert_eq!(decision.verdict, "revise");
    assert_eq!(decision.worker_mode, "context_focus");
    assert!(decision.hint.contains("edit sympy/core/numbers.py"));
    assert_eq!(decision.focus_files, vec!["sympy/core/numbers.py"]);
    assert_eq!(
        decision.required_checks,
        vec!["python -m pytest sympy/core/tests/test_power.py -q"]
    );
}

#[test]
fn subtracts_unchanged_preexisting_diff_blocks() {
    let before = "diff --git a/task.json b/task.json\nnew file mode 100644\n--- /dev/null\n+++ b/task.json\n@@ -0,0 +1,1 @@\n+{}\n";
    let after = format!(
        "{before}diff --git a/src/generated.rs b/src/generated.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/generated.rs\n@@ -0,0 +1,1 @@\n+pub fn generated() {{}}\n"
    );
    let filtered = diff_without_unchanged_blocks(&after, before);
    assert!(!filtered.contains("task.json"));
    assert!(filtered.contains("src/generated.rs"));
}

#[test]
fn init_manages_only_mixmod_local_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".codex")).unwrap();
    fs::write(root.join(".codex/config.toml"), "existing = true\n").unwrap();
    fs::write(root.join(LEGACY_OPENCODE_CONFIG), "{\"user\":true}\n").unwrap();

    init_project(root).unwrap();
    assert!(is_managed_file(&root.join(MIXMOD_CONFIG)));
    assert!(is_managed_file(&root.join(OPENCODE_CONFIG)));
    assert!(root.join(MIXMOD_CODEX_HOME).is_dir());
    assert!(!root.join(CODEX_INSTRUCTIONS).exists());
    assert!(!root.join(".codex/hooks.json").exists());
    assert!(!root.join(".codex/hooks").exists());
    assert!(root.join(".mixmod/backups").exists());
    assert_eq!(
        fs::read_to_string(root.join(".codex/config.toml")).unwrap(),
        "existing = true\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(LEGACY_OPENCODE_CONFIG)).unwrap(),
        "{\"user\":true}\n"
    );
}

#[test]
fn codex_exec_uses_mixmod_scoped_codex_home() {
    assert_eq!(
        codex_home_for_work_dir(Path::new("/tmp/work")),
        PathBuf::from("/tmp/work").join(MIXMOD_CODEX_HOME)
    );
}

#[test]
fn opencode_uses_mixmod_scoped_config() {
    assert_eq!(
        opencode_config_path(Path::new("/tmp/work")),
        PathBuf::from("/tmp/work").join(OPENCODE_CONFIG)
    );
}

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

    let receipt = run_mixmod_task(
        root,
        DelegationMode::Patch,
        &task,
        &root.join(".mixmod/runs/example"),
        &FakeRunner,
    )
    .unwrap();

    assert_eq!(receipt.status, "success");
    for artifact in [
        "receipt.json",
        "task.json",
        "report.md",
        "session.jsonl",
        "changes.patch",
        "tests.json",
        "metrics.json",
        "logs/opencode.stdout.txt",
        "logs/opencode.stderr.txt",
    ] {
        assert!(root.join(".mixmod/runs/example").join(artifact).exists());
    }
    let patch = fs::read_to_string(root.join(".mixmod/runs/example/changes.patch")).unwrap();
    assert!(patch.contains("src/generated.rs"));
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

    let receipt = run_mixmod_task(
        root,
        DelegationMode::Patch,
        &task,
        &root.join(".mixmod/runs/example"),
        &runner,
    )
    .unwrap();

    assert_eq!(receipt.status, "success");
    assert_eq!(runner.calls.load(AtomicOrdering::SeqCst), 2);
    let run_dir = root.join(".mixmod/runs/example");
    let patch = fs::read_to_string(run_dir.join("changes.patch")).unwrap();
    assert!(patch.contains("src/generated.rs"));
    assert!(run_dir.join("empty-patch-followup/task.json").exists());
    assert!(
        run_dir
            .join("empty-patch-followup/opencode-instructions.md")
            .exists()
    );
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
    impl OpenCodeRunner for NoEditRunner {
        fn run(&self, request: &OpenCodeRequest) -> Result<OpenCodeOutput> {
            Ok(OpenCodeOutput {
                command_for_metrics: vec!["fake-opencode".to_string()],
                opencode_segments: Vec::new(),
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
                worker_session_reused: request.resume_session_id.is_some(),
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

    let receipt = run_mixmod_task(
        root,
        DelegationMode::Patch,
        &task,
        &root.join(".mixmod/runs/example"),
        &NoEditRunner,
    )
    .unwrap();

    assert_eq!(receipt.status, "success");
    let metrics = read_json_file(&root.join(".mixmod/runs/example/metrics.json")).unwrap();
    assert_eq!(get_bool(&metrics, "expect_patch"), Some(false));
    assert_eq!(
        get_bool(&metrics, "empty_patch_followup_triggered"),
        Some(false)
    );
}

#[test]
fn opencode_instruction_includes_local_worker_self_check() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Worker support",
  "instructions": "Make the requested local change.",
  "files": ["src/lib.rs"],
  "tests": ["cargo test"]
}"#,
    )
    .unwrap();
    let (_, task_spec) = read_task_json(&task).unwrap();

    let instruction = build_opencode_instruction(
        DelegationMode::Patch,
        &task_spec,
        &task,
        &root.join(".mixmod/runs/example"),
    )
    .unwrap();

    assert!(instruction.contains("## Completion Self-Check"));
    assert!(instruction.contains("Did you complete every edit you intended to make?"));
    assert!(instruction.contains("If you intended checks or verification"));
    assert!(instruction.contains("Do not claim success if intended edits"));
    assert!(instruction.contains("## Output Contract"));
}

#[test]
fn as_given_worker_brief_uses_original_task_defaults() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "As-given handoff",
  "instructions": "Fix the checkout bug.",
  "files": ["checkout.py"],
  "tests": ["python -m unittest -q"],
  "constraints": ["Keep patch focused."],
  "acceptance": ["Tests pass."]
}"#,
    )
    .unwrap();

    let brief = json!({"handoff": "as_given"});
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();

    assert_eq!(get_string_array(&worker_task, "files"), vec!["checkout.py"]);
    assert_eq!(
        get_string_array(&worker_task, "tests"),
        vec!["python -m unittest -q"]
    );
    assert_eq!(
        get_string_array(&worker_task, "acceptance"),
        vec!["Tests pass."]
    );

    let instructions = get_str(&worker_task, "instructions").unwrap();
    assert!(instructions.contains("Codex message to OpenCode:"));
    assert!(instructions.contains("Proceed from the original task."));
    assert!(instructions.contains("Fix the checkout bug."));
    assert!(!instructions.contains("Objective:"));
    assert!(!instructions.contains("Implementation steps:"));
    assert_eq!(worker_task["context"]["worker_brief"], brief);
}

#[test]
fn worker_brief_prompt_prioritizes_compact_executable_handoff() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    atomic_write(
        &root.join("checkout.py"),
        b"def total(items):\n    return sum(items)\n",
    )
    .unwrap();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Checkout",
  "instructions": "Fix totals.",
  "files": ["checkout.py"],
  "tests": ["python -m unittest -q"]
}"#,
    )
    .unwrap();

    let prompt = frontier_worker_brief_prompt(root, &task).unwrap();

    assert!(prompt.contains("minimize frontier output"));
    assert!(prompt.contains("compact executable worker handoff"));
    assert!(prompt.contains("exact files, edit target, expected behavior, and checks"));
    assert!(prompt.contains(r#"Default to "guided""#));
    assert!(prompt.contains("Guided means terse and executable"));
    assert!(prompt.contains("target <=120 output tokens"));
    assert!(prompt.contains("one command-style message_to_worker"));
    assert!(prompt.contains("usually <=2"));
    assert!(prompt.contains("omit avoid and risk"));
    assert!(prompt.contains(r#""expect_patch": true"#));
    assert!(prompt.contains("Set false for investigation/no-change handoffs"));
    assert!(prompt.contains("setup rabbit holes"));
    assert!(prompt.contains("already names the relevant files, desired behavior, and checks"));
    assert!(prompt.contains(r#"{"handoff":"as_given"}"#));
    assert!(prompt.contains("omit empty fields"));
}

#[test]
fn agent_visible_task_strips_swebench_scoring_metadata() {
    let task = json!({
        "title": "SWE-bench Lite sympy__sympy-20212",
        "instructions": "Fix 0**-oo.",
        "files": [],
        "tests": [],
        "patch": "gold implementation diff",
        "test_patch": "hidden test diff",
        "hints_text": "look in numbers.py",
        "context": {
            "benchmark": "SWE-bench Lite",
            "dataset": "SWE-bench/SWE-bench_Lite",
            "split": "test",
            "instance_id": "sympy__sympy-20212",
            "repo": "sympy/sympy",
            "base_commit": "a106f4782a9dbe7f8fd16030f15401d977e03ae9",
            "version": "1.7",
            "fail_to_pass": "[\"test_zero\"]",
            "pass_to_pass": "[\"test_rational\"]",
            "candidate_pool": "codex-pass-pool-v1",
            "selection_rule": "keep only Codex-pass instances",
            "environment_setup_commit": "setup"
        }
    });

    let visible = agent_visible_task_value(&task);
    let text = serde_json::to_string(&visible).unwrap();

    assert!(text.contains("Fix 0**-oo."));
    assert!(text.contains("sympy__sympy-20212"));
    assert!(text.contains("sympy/sympy"));
    for forbidden in [
        "test_zero",
        "test_rational",
        "gold implementation diff",
        "hidden test diff",
        "look in numbers.py",
        "candidate_pool",
        "selection_rule",
        "environment_setup_commit",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} leaked in {text}");
    }
}

#[test]
fn codex_prompt_uses_agent_visible_task() {
    let task = json!({
        "title": "SWE-bench Lite sympy__sympy-20212",
        "instructions": "Fix 0**-oo.",
        "context": {
            "instance_id": "sympy__sympy-20212",
            "repo": "sympy/sympy",
            "fail_to_pass": "[\"test_zero\"]",
            "pass_to_pass": "[\"test_rational\"]"
        }
    });

    let prompt = codex_only_prompt(Path::new("/tmp/work"), &task).unwrap();

    assert!(prompt.contains("Fix 0**-oo."));
    assert!(prompt.contains("sympy__sympy-20212"));
    assert!(!prompt.contains("test_zero"));
    assert!(!prompt.contains("test_rational"));
    assert!(!prompt.contains("fail_to_pass"));
    assert!(!prompt.contains("pass_to_pass"));
}

#[test]
fn worktree_task_copy_is_agent_visible_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let source = root.join("full-task.json");
    let target = root.join("work/task.json");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    atomic_write(
        &source,
        br#"{
  "title": "SWE-bench Lite sympy__sympy-20212",
  "instructions": "Fix 0**-oo.",
  "context": {
    "instance_id": "sympy__sympy-20212",
    "repo": "sympy/sympy",
    "fail_to_pass": "[\"test_zero\"]",
    "pass_to_pass": "[\"test_rational\"]"
  }
}"#,
    )
    .unwrap();

    write_agent_visible_task_file(&source, &target).unwrap();
    let written = fs::read_to_string(&target).unwrap();

    assert!(written.contains("Fix 0**-oo."));
    assert!(!written.contains("test_zero"));
    assert!(!written.contains("test_rational"));
}

#[test]
fn worker_task_does_not_link_to_unsanitized_source_task() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "SWE-bench Lite sympy__sympy-20212",
  "instructions": "Fix 0**-oo.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": [],
  "context": {
    "instance_id": "sympy__sympy-20212",
    "repo": "sympy/sympy",
    "fail_to_pass": "[\"test_zero\"]",
    "pass_to_pass": "[\"test_rational\"]"
  }
}"#,
    )
    .unwrap();

    let brief = json!({"handoff": "as_given"});
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();
    let text = serde_json::to_string(&worker_task).unwrap();

    assert!(text.contains("Fix 0**-oo."));
    assert!(!text.contains("test_zero"));
    assert!(!text.contains("test_rational"));
    assert!(worker_task["context"].get("source_task").is_none());
}

#[test]
fn focused_worker_brief_overrides_files_and_adds_supplemental_checks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Focused handoff",
  "instructions": "Fix the checkout bug.",
  "files": ["catalog.py", "checkout.py"],
  "tests": ["python -m unittest -q"],
  "constraints": [],
  "acceptance": ["Tests pass."]
}"#,
    )
    .unwrap();

    let brief = json!({
        "handoff": "focused",
        "expect_patch": false,
        "supplement": "Preserve public return shapes.",
        "focus_files": ["checkout.py"],
        "must_check": ["VIP discount after line discounts"],
        "avoid": ["broad refactor"]
    });
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();

    assert_eq!(get_bool(&worker_task, "expect_patch"), Some(false));
    assert_eq!(get_string_array(&worker_task, "files"), vec!["checkout.py"]);
    assert_eq!(
        get_string_array(&worker_task, "acceptance"),
        vec!["VIP discount after line discounts"]
    );
    assert!(
        get_string_array(&worker_task, "constraints")
            .contains(&"Avoid: broad refactor".to_string())
    );
    let instructions = get_str(&worker_task, "instructions").unwrap();
    assert!(instructions.contains("Codex message to OpenCode:"));
    assert!(instructions.contains("Preserve public return shapes."));
    assert!(instructions.contains("Files:"));
}

#[test]
fn direct_worker_message_is_preserved() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "Direct handoff",
  "instructions": "Fix the checkout bug.",
  "files": ["catalog.py"],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();

    let brief = json!({
        "handoff": "guided",
        "expect_patch": true,
        "message_to_worker": "Investigate checkout totals and make the smallest safe fix.",
        "files": ["checkout.py"],
        "checks": ["VIP discount after line discounts"]
    });
    let worker_task_path = write_worker_brief_task(&task, &brief, &root.join("default")).unwrap();
    let worker_task = read_json_file(&worker_task_path).unwrap();

    assert_eq!(get_bool(&worker_task, "expect_patch"), Some(true));
    assert_eq!(get_string_array(&worker_task, "files"), vec!["checkout.py"]);
    assert_eq!(
        get_string_array(&worker_task, "acceptance"),
        vec!["VIP discount after line discounts"]
    );
    let instructions = get_str(&worker_task, "instructions").unwrap();
    assert!(instructions.contains(
        "Codex message to OpenCode:\nInvestigate checkout totals and make the smallest safe fix."
    ));
}

#[test]
fn feedback_reject_is_normalized_to_revise() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "verdict": "reject",
        "hint": "No patch was captured.",
        "focus_files": ["checkout.py"],
        "required_checks": ["python -m unittest -q"]
    }));

    assert_eq!(verdict, "revise");
    assert_eq!(get_str(&feedback, "verdict"), Some("revise"));
    assert_eq!(get_str(&feedback, "raw_verdict"), Some("reject"));
}

#[test]
fn feedback_stop_is_preserved_as_codex_stop() {
    let (feedback, verdict) = normalize_feedback_value(json!({
        "action": "stop",
        "message_to_worker": "No further local attempts.",
        "focus_files": [],
        "required_checks": []
    }));

    assert_eq!(verdict, "stop");
    assert_eq!(get_str(&feedback, "verdict"), Some("stop"));
    assert_eq!(get_str(&feedback, "action"), Some("stop"));
}

#[test]
fn revision_task_preserves_codex_focus_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = FrontierFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "continue".to_string(),
        hint: "Update the discount code and its test.".to_string(),
        focus_files: vec!["checkout.py".to_string(), "test_checkout.py".to_string()],
        required_checks: vec!["python -m unittest -q".to_string()],
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
    };

    let path = write_revision_task(&task, &root.join("default"), "demo", &decision, 1).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(
        get_string_array(&revision, "files"),
        vec!["checkout.py", "test_checkout.py"]
    );
    assert_eq!(
        get_string_array(&revision, "acceptance"),
        vec!["python -m unittest -q"]
    );
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("Message to OpenCode: Update the discount code"));
}

#[test]
fn context_focus_revision_task_uses_focused_prompt() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("task.json");
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = FrontierFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "context_focus".to_string(),
        hint: "Ignore dependency setup and edit the focused files first.".to_string(),
        focus_files: vec!["checkout.py".to_string()],
        required_checks: vec!["python -m unittest -q".to_string()],
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
    };

    let path = write_revision_task(&task, &root.join("default"), "demo", &decision, 2).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(get_str(&revision, "worker_mode"), Some("context_focus"));
    assert_eq!(get_string_array(&revision, "files"), vec!["checkout.py"]);
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("worker_mode=context_focus"));
    assert!(instructions.contains("fresh focused worker attempt"));
    assert!(instructions.contains("make the code/test edit first"));
}

#[test]
fn revision_task_keeps_mixmod_artifacts_out_of_repo_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let task = root.join("work/task.json");
    fs::create_dir_all(task.parent().unwrap()).unwrap();
    atomic_write(
        &task,
        br#"{
  "title": "demo",
  "instructions": "Fix the checkout bug.",
  "files": [],
  "tests": [],
  "constraints": [],
  "acceptance": []
}"#,
    )
    .unwrap();
    let decision = FrontierFeedbackTurn {
        feedback: json!({}),
        verdict: "revise".to_string(),
        worker_mode: "context_focus".to_string(),
        hint: "Use the latest focused task and edit the source file.".to_string(),
        focus_files: vec![
            "revision-task-3.json".to_string(),
            "sympy/core/power.py".to_string(),
            "../default/worker-brief.json".to_string(),
        ],
        required_checks: vec![],
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        input_bytes: 0,
        output_bytes: 0,
    };

    let path = write_revision_task(&task, &root.join("default"), "demo", &decision, 4).unwrap();
    let revision = read_json_file(&path).unwrap();

    assert_eq!(
        get_string_array(&revision, "files"),
        vec!["sympy/core/power.py"]
    );
    assert_eq!(
        get_string_array(&revision["context"], "mixmod_artifact_refs"),
        vec!["revision-task-3.json", "../default/worker-brief.json"]
    );
    let instructions = get_str(&revision, "instructions").unwrap();
    assert!(instructions.contains("Mixmod artifact references from Codex"));
    assert!(!instructions.contains("Focus files: [\"revision-task-3.json\""));
}

#[test]
fn experiment_report_handles_missing_telemetry() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    experiment_init(root, "demo", None).unwrap();
    let report = experiment_report(root, "demo").unwrap();
    assert!(report.contains("Exact token telemetry"));
    assert!(root.join(".mixmod/experiments/demo/report.md").exists());
}
