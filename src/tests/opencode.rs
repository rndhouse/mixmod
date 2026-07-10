use super::*;

#[test]
fn resumed_opencode_args_use_specific_session_without_title() {
    let args = vec![
        "run".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--model".to_string(),
        "llama.cpp/qwen/qwen3.6-27b".to_string(),
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
            "llama.cpp/qwen/qwen3.6-27b",
            "Do the task",
        ]
    );
}

#[test]
fn default_opencode_args_pin_mixmod_worker_agent() {
    let args = OpenCodeConfig::default().args;

    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--agent" && pair[1] == MIXMOD_OPENCODE_AGENT)
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--model" && pair[1] == "{model_arg}")
    );
}

#[test]
fn default_backend_probe_uses_configured_opencode_base_url() {
    let command = effective_backend_command_for_base_url(
        "curl -fsS http://127.0.0.1:8080/v1/models",
        Some("http://192.168.1.124:8080/v1/"),
    );

    assert_eq!(
        command,
        "curl --noproxy '*' -fsS http://192.168.1.124:8080/v1/models"
    );
}

fn test_opencode_request(root: &Path) -> AgentRequest {
    let out_dir = state_layout(root).runs().join("test");
    AgentRequest {
        root: root.to_path_buf(),
        mode: DelegationMode::Patch,
        task_path: root.join("task.json"),
        out_dir: out_dir.clone(),
        instruction_path: out_dir.join("opencode-instructions.md"),
        instruction: "FULL ORIGINAL INSTRUCTION".to_string(),
        session_id: "opencode-session-test".to_string(),
        resume_session_id: None,
        require_local: false,
        supervisor_advisor: None,
    }
}

#[test]
fn final_backend_probe_can_verify_short_local_worker_turn() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let fake_opencode = root.join("fake-opencode.sh");
    let marker = root.join("worker-done");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "models" ]; then
  echo "llama.cpp/qwen/qwen3.6-27b"
  exit 0
fi
sleep 0.2
touch "{}"
echo "done"
exit 0
"#,
        marker.display()
    );
    atomic_write(&fake_opencode, script.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fake_opencode).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake_opencode, perms).unwrap();
    }

    let mut request = test_opencode_request(root);
    request.require_local = true;
    let mut config = OpenCodeConfig::default();
    config.local_verification.enabled = true;
    config.local_verification.gpu_command.clear();
    config.local_verification.backend_command = format!(
        "test -f {} && printf 'qwen/qwen3.6-27b\\n'",
        marker.display()
    );
    config.heartbeat_seconds = 60;
    config.worker_timeout_seconds = 10;
    config.idle_timeout_seconds = 0;
    let selection = OpenCodeModelSelection {
        provider: "llama.cpp".to_string(),
        model: "qwen/qwen3.6-27b".to_string(),
        model_arg: "llama.cpp/qwen/qwen3.6-27b".to_string(),
        require_local: true,
    };

    let output = run_with_local_verification(
        fake_opencode.to_str().unwrap(),
        &["run".to_string(), "Do the task".to_string()],
        &request,
        &config,
        &selection,
    )
    .unwrap();

    assert_eq!(output.exit_status, Some(0));
    assert!(output.backend_activity_observed);
    assert!(output.local_inference_verified);
    let backend_status =
        fs::read_to_string(request.out_dir.join("logs/backend-status.txt")).unwrap();
    assert!(backend_status.contains("--- final sample ---"));
}

#[test]
fn interrupt_continue_args_resume_session_with_only_control_message() {
    let temp = TempDir::new().unwrap();
    let request = test_opencode_request(temp.path());
    let args = vec![
        "run".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--model".to_string(),
        "llama.cpp/qwen/qwen3.6-27b".to_string(),
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
            "llama.cpp/qwen/qwen3.6-27b",
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
        "llama.cpp/qwen/qwen3.6-27b".to_string(),
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
            "llama.cpp/qwen/qwen3.6-27b",
            "Fresh focus.",
        ]
    );
}

struct StaticAbortWorkerTurnAdvisor;

impl SupervisorAdvisor for StaticAbortWorkerTurnAdvisor {
    fn advise(&self, snapshot: &LiveWorkerSnapshot) -> Result<Option<Value>> {
        assert_eq!(snapshot.new_delta_bytes, 0);
        Ok(Some(json!({
            "action": "abort_worker_turn",
            "message_to_worker": "Abort this stalled turn.",
            "risk": "test_abort_worker_turn"
        })))
    }
}

#[test]
fn live_advisor_control_aborts_running_opencode() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let fake_opencode = root.join("fake-opencode.sh");
    let calls_path = root.join("calls.txt");
    let script = format!(
        r#"#!/bin/sh
printf 'cmd=%s args=%s\n' "$1" "$*" >> "{}"
if [ "$1" = "models" ]; then
  echo "llama.cpp/qwen/qwen3.6-27b"
  exit 0
fi
if [ "$1" = "db" ]; then
  echo '[{{"id":"ses_fake"}}]'
  exit 0
fi
echo "initial"
exec sleep 30
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

    let out_dir = state_layout(root).runs().join("live-advisor-abort-test");
    let advisor: std::sync::Arc<dyn SupervisorAdvisor> =
        std::sync::Arc::new(StaticAbortWorkerTurnAdvisor);
    let request = AgentRequest {
        root: root.to_path_buf(),
        mode: DelegationMode::Patch,
        task_path: root.join("task.json"),
        out_dir: out_dir.clone(),
        instruction_path: out_dir.join("opencode-instructions.md"),
        instruction: "FULL ORIGINAL INSTRUCTION".to_string(),
        session_id: "opencode-session-test".to_string(),
        resume_session_id: None,
        require_local: false,
        supervisor_advisor: Some(advisor),
    };
    let mut config = OpenCodeConfig::default();
    config.local_verification.enabled = false;
    config.heartbeat_seconds = 1;
    config.worker_timeout_seconds = 10;
    config.idle_timeout_seconds = 0;
    let selection = OpenCodeModelSelection {
        provider: "llama.cpp".to_string(),
        model: "qwen/qwen3.6-27b".to_string(),
        model_arg: "llama.cpp/qwen/qwen3.6-27b".to_string(),
        require_local: false,
    };

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

    assert!(output.interrupted_by_supervisor);
    assert_eq!(
        output.supervisor_control_action.as_deref(),
        Some("abort_worker_turn")
    );
    assert_eq!(output.supervisor_control_events.len(), 1);
    assert_eq!(
        output.supervisor_control_events[0].risk.as_str(),
        "test_abort_worker_turn"
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
  echo "llama.cpp/qwen/qwen3.6-27b"
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

    let out_dir = state_layout(root).runs().join("control-send-test");
    let request = AgentRequest {
        root: root.to_path_buf(),
        mode: DelegationMode::Patch,
        task_path: root.join("task.json"),
        out_dir: out_dir.clone(),
        instruction_path: out_dir.join("opencode-instructions.md"),
        instruction: "FULL ORIGINAL INSTRUCTION".to_string(),
        session_id: "opencode-session-test".to_string(),
        resume_session_id: None,
        require_local: false,
        supervisor_advisor: None,
    };
    let mut config = OpenCodeConfig::default();
    config.local_verification.enabled = false;
    config.heartbeat_seconds = 1;
    config.worker_timeout_seconds = 10;
    config.idle_timeout_seconds = 0;
    let selection = OpenCodeModelSelection {
        provider: "llama.cpp".to_string(),
        model: "qwen/qwen3.6-27b".to_string(),
        model_arg: "llama.cpp/qwen/qwen3.6-27b".to_string(),
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
