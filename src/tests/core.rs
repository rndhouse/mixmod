use super::*;

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
fn summary_reports_captured_patch_when_supervisor_needed() {
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
        &stats,
    );

    assert!(summary.contains("with 1 file(s) and 2 line(s) changed"));
    assert!(!summary.contains("no patch was captured"));
}

#[test]
fn summary_reports_accumulated_patch_when_latest_delta_is_empty() {
    let output = minimal_opencode_output();
    let latest_delta = PatchStats {
        files: vec![],
        changed_line_count: 0,
        added_lines: 0,
        removed_lines: 0,
    };
    let worktree_stats = PatchStats {
        files: vec!["django/db/models/deletion.py".to_string()],
        changed_line_count: 1,
        added_lines: 1,
        removed_lines: 0,
    };

    let summary = build_run_summary(
        "needs_supervisor",
        DelegationMode::Patch,
        &output,
        &latest_delta,
        &worktree_stats,
    );

    assert!(summary.contains("no new delta"));
    assert!(summary.contains("current worktree patch has 1 file(s)"));
    assert!(!summary.contains("no patch was captured"));
}

#[test]
fn worker_context_signals_detect_context_overflow_errors() {
    let stdout = br#"
{"type":"reasoning","part":{"text":"still thinking"}}
{"type":"error","error":{"name":"ContextOverflowError","data":{"message":"request (33126 tokens) exceeds the available context size (32768 tokens), try increasing it"}}}
plain backend line: request (34808 tokens) exceeds the available context size (32768 tokens)
"#;

    let signals = worker_context_signals(stdout);

    assert_eq!(signals.context_overflow_count, 2);
    assert_eq!(
        signals.context_overflow_last_message.as_deref(),
        Some(
            "plain backend line: request (34808 tokens) exceeds the available context size (32768 tokens)"
        )
    );
}

#[test]
fn worker_session_token_peak_reads_open_code_step_tokens() {
    let stdout = br#"
not json
{"type":"step_finish","part":{"tokens":{"total":1200,"input":10}}}
{"type":"reasoning","part":{"text":"ignore"}}
{"type":"step_finish","part":{"tokens":{"total":25730}}}
"#;

    assert_eq!(worker_session_token_peak(stdout), Some(25730));
}

#[test]
fn worker_session_token_peak_ignores_stdout_without_token_events() {
    let stdout = br#"
{"type":"reasoning","part":{"text":"still thinking"}}
plain backend line
"#;

    assert_eq!(worker_session_token_peak(stdout), None);
}

#[test]
fn codex_usage_reads_rollout_total_token_usage() {
    let stdout = br#"
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2,"reasoning_output_tokens":1,"total_tokens":12}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":30,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":3,"total_tokens":35}}}}
"#;

    let usage = crate::harness::codex::codex_usage_from_jsonl(stdout);

    assert_eq!(usage.input_tokens, 30);
    assert_eq!(usage.cached_input_tokens, 20);
    assert_eq!(usage.output_tokens, 5);
    assert_eq!(usage.reasoning_tokens, 3);
    assert_eq!(usage.total_tokens, 35);
}

#[test]
fn codex_usage_reads_app_server_total_fallback() {
    let stdout = br#"
{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"inputTokens":1,"cachedInputTokens":0,"outputTokens":1,"reasoningOutputTokens":0,"totalTokens":2},"total":{"inputTokens":40,"cachedInputTokens":30,"outputTokens":6,"reasoningOutputTokens":2,"totalTokens":46}}}}
"#;

    let usage = crate::harness::codex::codex_usage_from_jsonl(stdout);

    assert_eq!(usage.input_tokens, 40);
    assert_eq!(usage.cached_input_tokens, 30);
    assert_eq!(usage.output_tokens, 6);
    assert_eq!(usage.reasoning_tokens, 2);
    assert_eq!(usage.total_tokens, 46);
}

#[test]
fn codex_app_server_turn_delta_uses_cumulative_total_not_last_request() {
    let token_usage = json!({
        "last": {
            "inputTokens": 1,
            "cachedInputTokens": 0,
            "outputTokens": 1,
            "reasoningOutputTokens": 0,
            "totalTokens": 2
        },
        "total": {
            "inputTokens": 40,
            "cachedInputTokens": 30,
            "outputTokens": 6,
            "reasoningOutputTokens": 2,
            "totalTokens": 46
        }
    });
    let previous = crate::harness::codex::CodexUsage {
        input_tokens: 10,
        cached_input_tokens: 8,
        output_tokens: 3,
        reasoning_tokens: 1,
        total_tokens: 13,
    };

    let cumulative =
        crate::harness::codex::codex_app_server_cumulative_usage(&token_usage).unwrap();
    let delta = cumulative.delta_since(&previous);

    assert_eq!(delta.input_tokens, 30);
    assert_eq!(delta.cached_input_tokens, 22);
    assert_eq!(delta.output_tokens, 3);
    assert_eq!(delta.reasoning_tokens, 1);
    assert_eq!(delta.total_tokens, 33);
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
            Some("llama.cpp/qwen/qwen3.6-27b".to_string()),
        )
        .with_worker_backend(Some(WorkerBackend::OpenCode)),
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
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--worker-model" && pair[1] == "llama.cpp/qwen/qwen3.6-27b")
    );
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--worker-backend" && pair[1] == "opencode")
    );
}

#[test]
fn debug_commands_are_gated_by_default() {
    let error = ensure_debug_command_enabled("mixmod run-worker").unwrap_err();

    assert!(error.to_string().contains("mixmod exec"));
    assert!(error.to_string().contains("MIXMOD_DEBUG_COMMANDS=1"));
}

#[test]
fn exec_command_is_public_cli_surface() {
    let cli = Cli::try_parse_from(["mixmod", "exec", "Fix", "checkout", "totals"]).unwrap();

    match cli.command {
        Commands::Exec {
            task,
            resume_session,
            supervisor_model,
            worker_model,
            worker_backend,
            supervisor_init,
            stop_after_first_worker,
            no_require_local,
            prompt,
        } => {
            assert!(task.is_none());
            assert!(resume_session.is_none());
            assert!(supervisor_model.is_none());
            assert!(worker_model.is_none());
            assert!(worker_backend.is_none());
            assert!(supervisor_init.is_none());
            assert!(!stop_after_first_worker);
            assert!(!no_require_local);
            assert_eq!(prompt, vec!["Fix", "checkout", "totals"]);
        }
        _ => panic!("expected exec command"),
    }

    let cli = Cli::try_parse_from(["mixmod", "exec", "--task", "task.json"]).unwrap();
    match cli.command {
        Commands::Exec { task, prompt, .. } => {
            assert_eq!(task, Some(PathBuf::from("task.json")));
            assert!(prompt.is_empty());
        }
        _ => panic!("expected exec command"),
    }

    assert!(Cli::try_parse_from(["mixmod", "delegate"]).is_err());
    assert!(
        Cli::try_parse_from(["mixmod", "exec", "--task", "task.json", "--require-local",]).is_err()
    );
    let cli = Cli::try_parse_from(["mixmod", "tool", "ask", "--prompt", "review the final diff"])
        .unwrap();
    match cli.command {
        Commands::Tool {
            command: ToolCommand::Ask { prompt, args },
        } => {
            assert_eq!(prompt.as_deref(), Some("review the final diff"));
            assert!(args.is_empty());
        }
        _ => panic!("expected tool ask"),
    }
    let cli = Cli::try_parse_from([
        "mixmod", "tool", "ask", "--", "review", "the", "final", "diff",
    ])
    .unwrap();
    match cli.command {
        Commands::Tool {
            command: ToolCommand::Ask { prompt, args },
        } => {
            assert!(prompt.is_none());
            assert_eq!(args, vec!["review", "the", "final", "diff"]);
        }
        _ => panic!("expected tool ask"),
    }
    let cli = Cli::try_parse_from([
        "mixmod",
        "tool",
        "run-command",
        "--command",
        "git status --short",
    ])
    .unwrap();
    match cli.command {
        Commands::Tool {
            command: ToolCommand::RunCommand { command, args },
        } => {
            assert_eq!(command.as_deref(), Some("git status --short"));
            assert!(args.is_empty());
        }
        _ => panic!("expected tool run-command"),
    }
    let cli = Cli::try_parse_from([
        "mixmod",
        "tool",
        "run-command",
        "--",
        "git",
        "status",
        "--short",
    ])
    .unwrap();
    match cli.command {
        Commands::Tool {
            command: ToolCommand::RunCommand { command, args },
        } => {
            assert!(command.is_none());
            assert_eq!(args, vec!["git", "status", "--short"]);
        }
        _ => panic!("expected tool run-command"),
    }
    let cli = Cli::try_parse_from([
        "mixmod",
        "exec",
        "--task",
        "task.json",
        "--no-require-local",
    ])
    .unwrap();
    match cli.command {
        Commands::Exec {
            no_require_local, ..
        } => assert!(no_require_local),
        _ => panic!("expected exec command"),
    }
    assert!(
        Cli::try_parse_from([
            "mixmod",
            "exec",
            "--task",
            "task.json",
            "--out",
            "runs/demo",
        ])
        .is_err()
    );
}

#[test]
fn exec_accepts_supervisor_and_worker_model_flags() {
    let cli = Cli::try_parse_from([
        "mixmod",
        "exec",
        "--supervisor-model",
        "gpt-5.5:high",
        "--worker-backend",
        "codex",
        "--worker-model",
        "gpt-5.4:medium",
        "--supervisor-init",
        "investigate",
        "--stop-after-first-worker",
        "Fix checkout totals.",
    ])
    .unwrap();

    match cli.command {
        Commands::Exec {
            supervisor_model,
            worker_model,
            worker_backend,
            supervisor_init,
            stop_after_first_worker,
            no_require_local,
            prompt,
            ..
        } => {
            assert_eq!(supervisor_model, Some("gpt-5.5:high".to_string()));
            assert_eq!(worker_backend, Some(WorkerBackend::Codex));
            assert_eq!(worker_model, Some("gpt-5.4:medium".to_string()));
            assert_eq!(supervisor_init, Some(SupervisorInitMode::Investigate));
            assert!(stop_after_first_worker);
            assert!(!no_require_local);
            assert_eq!(prompt, vec!["Fix checkout totals."]);
        }
        _ => panic!("expected exec command"),
    }
}

#[test]
fn experiment_run_default_accepts_model_override_flags() {
    let cli = Cli::try_parse_from([
        "mixmod",
        "experiment",
        "run-default",
        "deepswe",
        "--supervisor-model",
        "gpt-5.5:high",
        "--worker-backend",
        "opencode",
        "--worker-model",
        "openrouter/qwen/qwen3.6-flash",
        "--supervisor-init",
        "investigate",
        "--stop-after-first-worker",
    ])
    .unwrap();

    match cli.command {
        Commands::Experiment {
            command:
                ExperimentCommand::RunDefault {
                    name,
                    require_local,
                    supervisor_model,
                    worker_model,
                    worker_backend,
                    supervisor_init,
                    stop_after_first_worker,
                },
        } => {
            assert_eq!(name, "deepswe");
            assert!(!require_local);
            assert_eq!(supervisor_model, Some("gpt-5.5:high".to_string()));
            assert_eq!(worker_backend, Some(WorkerBackend::OpenCode));
            assert_eq!(
                worker_model,
                Some("openrouter/qwen/qwen3.6-flash".to_string())
            );
            assert_eq!(supervisor_init, Some(SupervisorInitMode::Investigate));
            assert!(stop_after_first_worker);
        }
        _ => panic!("expected experiment run-default command"),
    }
}

#[test]
fn exec_task_resolution_writes_prompt_tasks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let task_path = resolve_exec_task(
        root,
        None,
        vec!["Fix".to_string(), "checkout totals.".to_string()],
    )
    .unwrap();
    assert!(task_path.starts_with(state_layout(root).tasks()));
    assert!(!root.join(".mixmod").exists());
    let task = read_json_file(&task_path).unwrap();
    assert_eq!(get_str(&task, "title"), Some("Fix checkout totals."));
    assert_eq!(get_str(&task, "instructions"), Some("Fix checkout totals."));

    assert!(
        resolve_exec_task(
            root,
            Some(PathBuf::from("task.json")),
            vec!["Fix".to_string()]
        )
        .unwrap_err()
        .to_string()
        .contains("not both")
    );
    assert!(
        resolve_exec_task(root, None, Vec::new())
            .unwrap_err()
            .to_string()
            .contains("provide a prompt")
    );
}

#[test]
fn model_overrides_apply_supervisor_and_worker_models() {
    let mut config = MixmodConfig::default();

    ModelOverrides::new(
        Some("gpt-5.5:xhigh".to_string()),
        Some("llama.cpp/qwen/qwen3.6-27b".to_string()),
    )
    .apply_to_config(&mut config)
    .unwrap();

    assert_eq!(config.supervisor.model, "gpt-5.5");
    assert_eq!(config.supervisor.reasoning_effort, "xhigh");
    assert_eq!(config.opencode.provider, "llama.cpp");
    assert_eq!(config.opencode.model, "qwen/qwen3.6-27b");
    assert!(config.opencode.require_local);
    assert!(config.opencode.local_verification.enabled);
    assert!(
        config
            .opencode
            .model_aliases
            .get("qwen/qwen3.6-27b")
            .unwrap()
            .contains(&"llama.cpp/qwen/qwen3.6-27b".to_string())
    );
}

#[test]
fn openrouter_worker_override_selects_non_local_worker() {
    let mut config = MixmodConfig::default();

    ModelOverrides::new(None, Some("openrouter/qwen/qwen3.6-flash".to_string()))
        .apply_to_config(&mut config)
        .unwrap();

    assert_eq!(config.opencode.provider, "openrouter");
    assert_eq!(config.opencode.model, "qwen/qwen3.6-flash");
    assert!(!config.opencode.require_local);
    assert!(!config.opencode.local_verification.enabled);
    assert!(
        !config
            .opencode
            .local_providers
            .iter()
            .any(|provider| provider == "openrouter")
    );
    assert!(
        config
            .opencode
            .model_aliases
            .get("qwen/qwen3.6-flash")
            .unwrap()
            .contains(&"openrouter/qwen/qwen3.6-flash".to_string())
    );
}

#[test]
fn qwen_worker_profile_is_selected_by_default_and_alias() {
    let mut config = MixmodConfig::default();
    let guidance = config.worker_supervisor_guidance();

    assert_eq!(guidance.model, DEFAULT_OPENCODE_MODEL);
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("less capable than the supervisor"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("zero marginal GPT-token cost"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("reasoning before editing"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("concrete command-style local help"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("not final approval evidence"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("open-ended final diff review"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("concrete commands or probes"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("bounded snippets"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("multi-target"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("ordinary package tests are insufficient"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("uniquely named regression tests"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("worker_turn_shape=small_patch_slice"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("raw field names and resolved aliases"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("end-to-end semantics across slices"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("task-derived probes or focused tests"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("state the task contract"))
    );

    ModelOverrides::new(None, Some("llama.cpp/qwen/qwen3.6-27b".to_string()))
        .apply_to_config(&mut config)
        .unwrap();
    let guidance = config.worker_supervisor_guidance();

    assert_eq!(guidance.model, DEFAULT_OPENCODE_MODEL);
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("global environments"))
    );
}

#[test]
fn unknown_worker_model_has_no_default_guidance() {
    let mut config = MixmodConfig::default();
    ModelOverrides::new(
        None,
        Some("llama.cpp/unknown-local-model:latest".to_string()),
    )
    .apply_to_config(&mut config)
    .unwrap();

    assert!(config.worker_supervisor_guidance().is_empty());
}

#[test]
fn glm_worker_profile_is_selected_by_alias() {
    let mut config = MixmodConfig::default();
    ModelOverrides::new(None, Some("llama.cpp/glm-4.7-flash:Q4_K_M".to_string()))
        .apply_to_config(&mut config)
        .unwrap();

    let guidance = config.worker_supervisor_guidance();

    assert_eq!(guidance.model, "glm-4.7-flash:Q4_K_M");
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("rewrite or delete too much"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("worker_mode=continue"))
    );
}

#[test]
fn openrouter_glm_worker_profile_is_selected_by_alias() {
    let mut config = MixmodConfig::default();
    ModelOverrides::new(None, Some("openrouter/z-ai/glm-5.2".to_string()))
        .apply_to_config(&mut config)
        .unwrap();

    let guidance = config.worker_supervisor_guidance();

    assert_eq!(guidance.model, "openrouter/z-ai/glm-5.2");
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("over-investigate"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("resolve the implementation route"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("worker_turn_shape=bounded_feature_slice"))
    );
    assert!(
        guidance
            .guidance
            .iter()
            .any(|item| item.contains("trust that route"))
    );

    ModelOverrides::new(None, Some("z-ai/glm-5.2".to_string()))
        .apply_to_config(&mut config)
        .unwrap();
    let guidance = config.worker_supervisor_guidance();

    assert_eq!(guidance.model, "openrouter/z-ai/glm-5.2");
}

#[test]
fn model_overrides_apply_codex_worker_backend_and_model() {
    let mut config = MixmodConfig::default();

    ModelOverrides::new(
        Some("gpt-5.5:xhigh".to_string()),
        Some("gpt-5.4:medium".to_string()),
    )
    .with_worker_backend(Some(WorkerBackend::Codex))
    .apply_to_config(&mut config)
    .unwrap();

    assert_eq!(config.worker.backend, WorkerBackend::Codex);
    assert_eq!(config.supervisor.model, "gpt-5.5");
    assert_eq!(config.supervisor.reasoning_effort, "xhigh");
    assert_eq!(config.codex_worker.model, "gpt-5.4");
    assert_eq!(config.codex_worker.reasoning_effort, "medium");
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
fn codex_supervision_turns_use_workspace_write_app_server_policy() {
    let policy = CodexSandbox::WorkspaceWrite.as_turn_policy(Path::new("/tmp/work"));

    assert_eq!(
        CodexSandbox::WorkspaceWrite.as_thread_arg(),
        "workspace-write"
    );
    assert_eq!(get_str(&policy, "type"), Some("workspaceWrite"));
    assert_eq!(
        get_string_array(&policy, "writableRoots"),
        vec!["/tmp/work"]
    );
    assert_eq!(get_bool(&policy, "networkAccess"), Some(false));
}

#[test]
fn supervisor_reuse_metrics_are_derived_from_thread_ids() {
    let sample = |thread_id: &str, turn_id: &str| {
        SupervisorFeedbackTurn {
            feedback: json!({}),
            verdict: "approve".to_string(),
            worker_mode: "continue".to_string(),
            patch_decision: "accept_current".to_string(),
            hint: String::new(),
            revision_handoff: RevisionHandoff::default(),
            focus_files: vec![],
            required_checks: vec![],
            input_tokens: 1,
            output_tokens: 1,
            reasoning_tokens: 0,
            total_tokens: 2,
            cached_input_tokens: 0,
            input_bytes: 10,
            output_bytes: 20,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            token_usage_comparable: true,
        }
        .usage_sample()
    };

    let fresh_per_turn = vec![sample("thread-a", "turn-a"), sample("thread-b", "turn-b")];
    let fresh_usage = aggregate_supervisor_usage(&fresh_per_turn);
    assert_eq!(fresh_usage.turn_count, 2);
    assert_eq!(fresh_usage.thread_count(), 2);
    assert!(!fresh_usage.session_reused());
    assert_eq!(fresh_usage.thread_reuse_count(), 0);

    let reused_thread = vec![sample("thread-a", "turn-a"), sample("thread-a", "turn-b")];
    let reused_usage = aggregate_supervisor_usage(&reused_thread);
    assert_eq!(reused_usage.turn_count, 2);
    assert_eq!(reused_usage.thread_count(), 1);
    assert!(reused_usage.session_reused());
    assert_eq!(reused_usage.thread_reuse_count(), 1);
}

#[test]
fn codex_only_baseline_can_write_workspace_files() {
    let policy = CodexSandbox::WorkspaceWrite.as_turn_policy(Path::new("/tmp/work"));

    assert_eq!(
        CodexSandbox::WorkspaceWrite.as_thread_arg(),
        "workspace-write"
    );
    assert_eq!(get_str(&policy, "type"), Some("workspaceWrite"));
    assert_eq!(
        get_string_array(&policy, "writableRoots"),
        vec!["/tmp/work"]
    );
    assert_eq!(get_bool(&policy, "networkAccess"), Some(false));
}

#[test]
fn codex_app_server_can_run_without_inner_sandbox() {
    let policy = CodexSandbox::DangerFullAccess.as_turn_policy(Path::new("/tmp/work"));

    assert_eq!(
        CodexSandbox::DangerFullAccess.as_thread_arg(),
        "danger-full-access"
    );
    assert_eq!(get_str(&policy, "type"), Some("dangerFullAccess"));
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
