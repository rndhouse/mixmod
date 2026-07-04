use crate::*;

pub(crate) fn live_status(root: &Path, run: &Path, json_output: bool) -> Result<()> {
    let run_dir = absolutize(root, run);
    let status_path = run_dir.join(LIVE_STATUS_FILE);
    let heartbeat_path = run_dir.join("logs/heartbeat.jsonl");
    let stdout_path = run_dir.join("logs/opencode.stdout.txt");
    let stderr_path = run_dir.join("logs/opencode.stderr.txt");
    let status = read_json_file(&status_path).unwrap_or_else(|_| {
        json!({
            "status": "unavailable",
            "status_file": display_path(root, &status_path),
            "note": "run status is written only while a Mixmod worker run is active"
        })
    });
    let receipt = read_json_file(&run_dir.join("receipt.json")).ok();
    let metrics = read_json_file(&run_dir.join("metrics.json")).ok();
    let output = json!({
        "run_dir": display_path(root, &run_dir),
        "status": status,
        "receipt": receipt,
        "metrics": metrics,
        "tails": {
            "heartbeat": tail_text(&heartbeat_path, 3000),
            "stdout": tail_text(&stdout_path, 3000),
            "stderr": tail_text(&stderr_path, 5000)
        },
        "control": {
            "path": display_path(root, &run_dir.join(SUPERVISOR_CONTROL_FILE)),
            "actions": ["wait", "interrupt_continue", "interrupt_context_focus", "stop"]
        }
    });
    if json_output {
        let output = serde_json::to_string_pretty(&output)
            .context("failed to serialize control status output")?;
        println!("{output}");
    } else {
        let status_value = output.get("status").unwrap_or(&Value::Null);
        println!("run: {}", display_path(root, &run_dir));
        println!(
            "status: {}",
            get_str(status_value, "status").unwrap_or("unknown")
        );
        println!(
            "elapsed_ms: {}",
            get_u64(status_value, "elapsed_ms")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unavailable".to_string())
        );
        println!(
            "stdout_bytes: {}",
            get_u64(status_value, "stdout_bytes").unwrap_or(0)
        );
        println!(
            "stderr_bytes: {}",
            get_u64(status_value, "stderr_bytes").unwrap_or(0)
        );
        println!(
            "last_output_age_ms: {}",
            get_u64(status_value, "last_output_age_ms").unwrap_or(0)
        );
        println!(
            "gpu_activity_observed: {}",
            get_bool(status_value, "gpu_activity_observed")
                .map(yes_no)
                .unwrap_or("unknown")
        );
        println!(
            "backend_activity_observed: {}",
            get_bool(status_value, "backend_activity_observed")
                .map(yes_no)
                .unwrap_or("unknown")
        );
        println!(
            "control file: {}",
            display_path(root, &run_dir.join(SUPERVISOR_CONTROL_FILE))
        );
        println!("actions: wait, interrupt_continue, interrupt_context_focus, stop");
        println!("stderr tail:");
        println!("{}", tail_text(&stderr_path, 2000));
    }
    Ok(())
}

pub(crate) fn live_control(
    root: &Path,
    run: &Path,
    action: &str,
    message: Option<&str>,
    focus_files: &[String],
    required_checks: &[String],
    risk: Option<&str>,
) -> Result<()> {
    let run_dir = absolutize(root, run);
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;
    let action = normalize_supervisor_control_action(Some(action));
    let worker_mode = normalize_supervisor_control_worker_mode(&action, None);
    let control = SupervisorControlCommand {
        timestamp: Utc::now().to_rfc3339(),
        action,
        worker_mode,
        message_to_worker: message.unwrap_or("").to_string(),
        focus_files: focus_files.to_vec(),
        required_checks: required_checks.to_vec(),
        risk: risk.unwrap_or("").to_string(),
        source: "mixmod control send".to_string(),
    };
    let control_path = run_dir.join(SUPERVISOR_CONTROL_FILE);
    write_pretty_json(&control_path, &control, "supervisor control")?;
    println!(
        "wrote supervisor control: {}",
        display_path(root, &control_path)
    );
    println!("action: {}", control.action);
    if let Some(message) = message.filter(|value| !value.trim().is_empty()) {
        println!("message_to_worker: {message}");
    }
    Ok(())
}

pub(crate) fn supervise_mixmod_task(
    root: &Path,
    mode: DelegationMode,
    task_arg: &Path,
    out_arg: &Path,
    require_local: bool,
    resume_session: Option<String>,
    model_overrides: ModelOverrides,
) -> Result<()> {
    let task_path = absolutize(root, task_arg);
    let out_dir = absolutize(root, out_arg);
    let logs_dir = out_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;

    let exe = env::current_exe().context("failed to resolve current mixmod executable")?;
    let stdout_log = logs_dir.join("mixmod-run.stdout.txt");
    let stderr_log = logs_dir.join("mixmod-run.stderr.txt");
    let stdout_file = File::create(&stdout_log)
        .with_context(|| format!("failed to create {}", stdout_log.display()))?;
    let stderr_file = File::create(&stderr_log)
        .with_context(|| format!("failed to create {}", stderr_log.display()))?;

    let args = supervise_run_args(
        mode,
        &task_path,
        &out_dir,
        require_local,
        resume_session.as_deref(),
        &model_overrides,
    );
    let mut command = Command::new(&exe);
    command
        .args(&args)
        .current_dir(root)
        .env(DEBUG_COMMANDS_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));
    #[cfg(unix)]
    {
        command.process_group(0);
    }

    let child = command.spawn().with_context(|| {
        format!(
            "failed to start background Mixmod worker with {}",
            exe.display()
        )
    })?;
    let pid = child.id();
    drop(child);

    let command_for_metrics = std::iter::once(display_path(root, &exe))
        .chain(args.iter().cloned())
        .collect::<Vec<_>>();
    let run_display = display_path(root, &out_dir);
    let supervisor = json!({
        "started_at": Utc::now().to_rfc3339(),
        "status": "started",
        "pid": pid,
        "root": display_path(root, root),
        "mode": mode.to_string(),
        "task": display_path(root, &task_path),
        "run_dir": run_display,
        "require_local": require_local,
        "resume_session": resume_session,
        "model_overrides": model_overrides,
        "command": command_for_metrics,
        "internal_env": {
            "MIXMOD_DEBUG_COMMANDS": "1"
        },
        "stdout_log": display_path(root, &stdout_log),
        "stderr_log": display_path(root, &stderr_log),
        "control_status": format!("MIXMOD_DEBUG_COMMANDS=1 mixmod control status --run {run_display}"),
        "control_status_json": format!("MIXMOD_DEBUG_COMMANDS=1 mixmod control status --run {run_display} --json"),
        "control_continue": format!("MIXMOD_DEBUG_COMMANDS=1 mixmod control send --run {run_display} --action interrupt_continue --message '<message>'"),
        "control_context_focus": format!("MIXMOD_DEBUG_COMMANDS=1 mixmod control send --run {run_display} --action interrupt_context_focus --message '<message>'"),
        "compact_artifacts": [
            display_path(root, &out_dir.join("receipt.json")),
            display_path(root, &out_dir.join("report.md")),
            display_path(root, &out_dir.join("worktree.patch")),
            display_path(root, &out_dir.join("changes.patch")),
            display_path(root, &out_dir.join(INTERVENTIONS_JSONL)),
            display_path(root, &out_dir.join("metrics.json"))
        ],
        "notes": [
            "This command returns immediately so the same Codex session can inspect run status while the worker runs.",
            "stdout/stderr from the background Mixmod process are written under logs/.",
            "For manual debugging, set MIXMOD_DEBUG_COMMANDS=1 and use mixmod control send, or write control.json to steer or interrupt the worker."
        ]
    });
    write_pretty_json(
        &out_dir.join("supervisor.json"),
        &supervisor,
        "supervisor receipt",
    )?;

    println!("started background Mixmod worker");
    println!("pid: {pid}");
    println!("run: {run_display}");
    println!("stdout: {}", display_path(root, &stdout_log));
    println!("stderr: {}", display_path(root, &stderr_log));
    println!("debug status: MIXMOD_DEBUG_COMMANDS=1 mixmod control status --run {run_display}");
    println!(
        "debug control: MIXMOD_DEBUG_COMMANDS=1 mixmod control send --run {run_display} --action interrupt_continue --message '<message>'"
    );
    Ok(())
}

pub(crate) fn ensure_debug_command_enabled(command: &str) -> Result<()> {
    if env::var_os(DEBUG_COMMANDS_ENV).is_some() {
        return Ok(());
    }
    bail!(
        "{command} is hidden for normal Mixmod use. Use `mixmod exec \"describe the task\"` instead. Set {DEBUG_COMMANDS_ENV}=1 only for manual debugging."
    )
}

pub(crate) fn supervise_run_args(
    mode: DelegationMode,
    task_path: &Path,
    out_dir: &Path,
    require_local: bool,
    resume_session: Option<&str>,
    model_overrides: &ModelOverrides,
) -> Vec<String> {
    let mut args = vec![
        "run-worker".to_string(),
        mode.to_string(),
        "--task".to_string(),
        task_path.to_string_lossy().to_string(),
        "--out".to_string(),
        out_dir.to_string_lossy().to_string(),
    ];
    if require_local {
        args.push("--require-local".to_string());
    }
    if let Some(session) = resume_session.filter(|value| !value.trim().is_empty()) {
        args.push("--resume-session".to_string());
        args.push(session.to_string());
    }
    if let Some(model) = model_overrides
        .supervisor_model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        args.push("--supervisor-model".to_string());
        args.push(model.to_string());
    }
    if let Some(model) = model_overrides
        .worker_model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        args.push("--worker-model".to_string());
        args.push(model.to_string());
    }
    if let Some(worker_backend) = model_overrides.worker_backend {
        args.push("--worker-backend".to_string());
        args.push(worker_backend.as_str().to_string());
    }
    args
}
