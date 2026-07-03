use crate::*;

pub fn run_mixmod_task(
    root: &Path,
    mode: DelegationMode,
    task_arg: &Path,
    out_arg: &Path,
    runner: &dyn OpenCodeRunner,
) -> Result<Receipt> {
    run_mixmod_task_with_options(root, mode, task_arg, out_arg, runner, false)
}

pub fn run_mixmod_task_with_options(
    root: &Path,
    mode: DelegationMode,
    task_arg: &Path,
    out_arg: &Path,
    runner: &dyn OpenCodeRunner,
    require_local: bool,
) -> Result<Receipt> {
    run_mixmod_task_with_session(root, mode, task_arg, out_arg, runner, require_local, None)
}

pub(crate) fn run_mixmod_task_with_session(
    root: &Path,
    mode: DelegationMode,
    task_arg: &Path,
    out_arg: &Path,
    runner: &dyn OpenCodeRunner,
    require_local: bool,
    resume_session_id: Option<String>,
) -> Result<Receipt> {
    MixmodRun {
        root,
        mode,
        task_arg,
        out_arg,
        runner,
        require_local,
        resume_session_id,
    }
    .execute()
}

struct MixmodRun<'a> {
    root: &'a Path,
    mode: DelegationMode,
    task_arg: &'a Path,
    out_arg: &'a Path,
    runner: &'a dyn OpenCodeRunner,
    require_local: bool,
    resume_session_id: Option<String>,
}

impl MixmodRun<'_> {
    fn execute(self) -> Result<Receipt> {
        let Self {
            root,
            mode,
            task_arg,
            out_arg,
            runner,
            require_local,
            resume_session_id,
        } = self;
        let run_id = make_run_id("run");
        let task_path = absolutize(root, task_arg);
        let out_dir = absolutize(root, out_arg);
        let logs_dir = out_dir.join("logs");
        fs::create_dir_all(&logs_dir)
            .with_context(|| format!("failed to create {}", logs_dir.display()))?;

        let (task_value, task_spec) = read_task_json(&task_path)?;
        write_pretty_json(&out_dir.join("task.json"), &task_value, "run task")?;

        let session_id = make_run_id("opencode-session");
        let instruction = build_opencode_instruction(mode, &task_spec, &task_path, &out_dir)?;
        let instruction_path = out_dir.join("opencode-instructions.md");
        atomic_write(&instruction_path, instruction.as_bytes())?;

        let request = OpenCodeRequest {
            root: root.to_path_buf(),
            mode,
            task_path: task_path.clone(),
            out_dir: out_dir.clone(),
            instruction_path,
            instruction,
            session_id,
            resume_session_id,
            require_local,
        };

        let start_timestamp = Utc::now();
        let start = Instant::now();
        let before_diff = git_diff_with_untracked(root).ok();
        let mut output = runner.run(&request)?;
        let mut end_timestamp = Utc::now();
        let mut wall_clock_ms = start.elapsed().as_millis();
        let expect_patch = expect_patch_for_run(mode, &task_value);
        let mut empty_patch_followup = EmptyPatchFollowup::new();

        atomic_write(&logs_dir.join("opencode.stdout.txt"), &output.stdout)?;
        atomic_write(&logs_dir.join("opencode.stderr.txt"), &output.stderr)?;

        let mut notes = vec![
        "Exact Codex token telemetry is unavailable to this prototype unless provided manually."
            .to_string(),
        "OpenCode output is untrusted until Codex reviews the compact artifacts and patch."
            .to_string(),
    ];
        if before_diff.as_deref().unwrap_or_default().trim().is_empty() {
            notes.push("No pre-existing git diff was detected before OpenCode ran.".to_string());
        } else {
            notes.push(
            "A pre-existing git diff was present before OpenCode ran; changes.patch may include unrelated local changes."
                .to_string(),
        );
        }
        notes.push(
            "worktree.patch contains the accumulated current repository diff for supervisor review."
                .to_string(),
        );

        let mut worktree_patch = match git_diff_with_untracked(root) {
            Ok(after_diff) => after_diff,
            Err(error) => {
                notes.push(format!("Unable to capture git diff: {error}"));
                String::new()
            }
        };
        let mut patch = diff_without_unchanged_blocks(
            &worktree_patch,
            before_diff.as_deref().unwrap_or_default(),
        );
        if should_run_empty_patch_followup(mode, expect_patch, &output, &patch) {
            empty_patch_followup.triggered = true;
            empty_patch_followup.reason = Some(
                "patch-mode worker run expected a patch but no repository diff was captured"
                    .to_string(),
            );
            match run_empty_patch_followup(EmptyPatchFollowupRequest {
                root,
                mode,
                task: &task_spec,
                task_path: &task_path,
                out_dir: &out_dir,
                runner,
                require_local,
                original_request: &request,
                output: &output,
            }) {
                Ok((followup_output, followup_dir)) => {
                    empty_patch_followup.performed = true;
                    empty_patch_followup.run_dir = Some(display_path(root, &followup_dir));
                    notes.push(format!(
                        "Empty-patch follow-up was triggered and ran in {}.",
                        display_path(root, &followup_dir)
                    ));
                    output = merge_opencode_outputs(output, followup_output);
                    end_timestamp = Utc::now();
                    wall_clock_ms = start.elapsed().as_millis();
                    worktree_patch = match git_diff_with_untracked(root) {
                        Ok(after_diff) => after_diff,
                        Err(error) => {
                            notes.push(format!(
                                "Unable to capture git diff after empty-patch follow-up: {error}"
                            ));
                            String::new()
                        }
                    };
                    patch = diff_without_unchanged_blocks(
                        &worktree_patch,
                        before_diff.as_deref().unwrap_or_default(),
                    );
                    empty_patch_followup.patch_created = !patch.trim().is_empty();
                }
                Err(error) => {
                    empty_patch_followup.reason = Some(format!(
                        "empty-patch follow-up was triggered but could not run: {error}"
                    ));
                    notes.push(format!(
                        "Empty-patch follow-up was triggered but could not run: {error}"
                    ));
                }
            }
            atomic_write(&logs_dir.join("opencode.stdout.txt"), &output.stdout)?;
            atomic_write(&logs_dir.join("opencode.stderr.txt"), &output.stderr)?;
        }
        atomic_write(&out_dir.join("changes.patch"), patch.as_bytes())?;
        atomic_write(&out_dir.join("worktree.patch"), worktree_patch.as_bytes())?;
        if output.timed_out || output.idle_timed_out {
            atomic_write(&out_dir.join("partial.patch"), patch.as_bytes())?;
            notes.push(
            "OpenCode did not finish normally; partial.patch preserves the worktree diff captured after termination."
                .to_string(),
        );
        }
        let stats = patch_stats(&patch);
        let worktree_stats = patch_stats(&worktree_patch);

        let (test_status, test_results, tests_json) =
            run_task_tests(root, &logs_dir, &task_spec.tests)?;
        write_pretty_json(&out_dir.join("tests.json"), &tests_json, "test results")?;

        let session = build_session_jsonl(&start_timestamp, &end_timestamp, &output)?;
        atomic_write(&out_dir.join("session.jsonl"), session.as_bytes())?;

        let needs_supervisor = output.timed_out
            || output.idle_timed_out
            || output.interrupted_by_supervisor
            || (output.success
                && ((mode == DelegationMode::Patch && expect_patch && patch.trim().is_empty())
                    || test_status == "failed"));
        let status = if needs_supervisor {
            "needs_supervisor"
        } else if output.success {
            "success"
        } else {
            "failed"
        };
        let summary =
            build_run_summary(status, mode, &output, &stats, &worktree_stats, &test_status);
        let report = build_run_report(RunReportInput {
            status,
            mode,
            summary: &summary,
            task: &task_spec,
            output: &output,
            stats: &stats,
            worktree_stats: &worktree_stats,
            test_status: &test_status,
            test_results: &test_results,
            notes: &notes,
            root,
            out_dir: &out_dir,
        });
        atomic_write(&out_dir.join("report.md"), report.as_bytes())?;

        let compact_artifacts = [
            "receipt.json",
            "report.md",
            "worktree.patch",
            "changes.patch",
            "tests.json",
            "metrics.json",
        ];
        let report_bytes = file_len(&out_dir.join("report.md"))?;
        let patch_bytes = file_len(&out_dir.join("changes.patch"))?;
        let worktree_patch_bytes = file_len(&out_dir.join("worktree.patch"))?;
        let session_bytes = file_len(&out_dir.join("session.jsonl"))?;
        let mut metrics = RunMetrics {
            start_timestamp: start_timestamp.to_rfc3339(),
            end_timestamp: end_timestamp.to_rfc3339(),
            wall_clock_ms,
            opencode_command: output.command_for_metrics.clone(),
            opencode_segments: output.opencode_segments.clone(),
            opencode_exit_status: output.exit_status,
            opencode_provider: output.provider.clone(),
            opencode_model: output.model.clone(),
            opencode_model_arg: output.model_arg.clone(),
            opencode_session_label: output.session_label.clone(),
            opencode_session_id: output.session_id.clone(),
            opencode_resume_session_id: output.resume_session_id.clone(),
            worker_session_reused: output.worker_session_reused,
            interrupted_by_supervisor: output.interrupted_by_supervisor,
            supervisor_control_action: output.supervisor_control_action.clone(),
            supervisor_control_events: output.supervisor_control_events.clone(),
            opencode_timed_out: output.timed_out,
            opencode_idle_timed_out: output.idle_timed_out,
            heartbeat_count: output.heartbeat_count,
            expect_patch,
            empty_patch_followup_triggered: empty_patch_followup.triggered,
            empty_patch_followup_performed: empty_patch_followup.performed,
            empty_patch_followup_patch_created: empty_patch_followup.patch_created,
            empty_patch_followup_reason: empty_patch_followup.reason.clone(),
            empty_patch_followup_run_dir: empty_patch_followup.run_dir.clone(),
            require_local: output.require_local,
            local_inference_verified: output.local_inference_verified,
            gpu_activity_observed: output.gpu_activity_observed,
            backend_activity_observed: output.backend_activity_observed,
            verification_notes: output.verification_notes.clone(),
            stdout_bytes: output.stdout.len() as u64,
            stderr_bytes: output.stderr.len() as u64,
            report_bytes,
            patch_bytes,
            worktree_patch_bytes,
            session_bytes,
            test_status: test_status.clone(),
            test_commands: task_spec.tests.clone(),
            test_results,
            changed_file_count: stats.files.len(),
            changed_line_count: stats.changed_line_count,
            codex_token_usage: None,
            approximate_codex_input_bytes: None,
            approximate_codex_output_bytes: None,
            artifact_files_read_by_codex: compact_artifacts
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            notes,
        };
        write_pretty_json(&out_dir.join("metrics.json"), &metrics, "run metrics")?;

        let receipt = Receipt {
            run_id,
            status: status.to_string(),
            mode: mode.to_string(),
            summary,
            changed_files: stats.files.clone(),
            report: display_path(root, &out_dir.join("report.md")),
            patch: display_path(root, &out_dir.join("changes.patch")),
            worktree_patch: display_path(root, &out_dir.join("worktree.patch")),
            session: display_path(root, &out_dir.join("session.jsonl")),
            tests: display_path(root, &out_dir.join("tests.json")),
            metrics: display_path(root, &out_dir.join("metrics.json")),
            logs: display_path(root, &logs_dir),
        };
        write_pretty_json(&out_dir.join("receipt.json"), &receipt, "run receipt")?;
        let compact_total = compact_artifacts
            .iter()
            .filter_map(|name| file_len(&out_dir.join(name)).ok())
            .sum();
        metrics.approximate_codex_input_bytes = Some(compact_total);
        write_pretty_json(&out_dir.join("metrics.json"), &metrics, "run metrics")?;
        let compact_total_after_metrics_update = compact_artifacts
            .iter()
            .filter_map(|name| file_len(&out_dir.join(name)).ok())
            .sum();
        if compact_total_after_metrics_update != compact_total {
            metrics.approximate_codex_input_bytes = Some(compact_total_after_metrics_update);
            write_pretty_json(&out_dir.join("metrics.json"), &metrics, "run metrics")?;
        }

        println!(
            "Mixmod run {} wrote artifacts to {}",
            receipt.run_id,
            out_dir.display()
        );
        println!("status: {}", receipt.status);
        println!("compact artifacts:");
        for artifact in compact_artifacts {
            println!("  {}", display_path(root, &out_dir.join(artifact)));
        }
        Ok(receipt)
    }
}

#[derive(Debug)]
struct EmptyPatchFollowup {
    triggered: bool,
    performed: bool,
    patch_created: bool,
    reason: Option<String>,
    run_dir: Option<String>,
}

impl EmptyPatchFollowup {
    fn new() -> Self {
        Self {
            triggered: false,
            performed: false,
            patch_created: false,
            reason: None,
            run_dir: None,
        }
    }
}

fn expect_patch_for_run(mode: DelegationMode, task: &Value) -> bool {
    if mode != DelegationMode::Patch {
        return false;
    }
    get_bool(task, "expect_patch")
        .or_else(|| {
            task.get("context")
                .and_then(|context| get_bool(context, "expect_patch"))
        })
        .or_else(|| {
            task.get("context")
                .and_then(|context| context.get("worker_brief"))
                .and_then(|brief| get_bool(brief, "expect_patch"))
        })
        .unwrap_or(true)
}

fn should_run_empty_patch_followup(
    mode: DelegationMode,
    expect_patch: bool,
    output: &OpenCodeOutput,
    patch: &str,
) -> bool {
    mode == DelegationMode::Patch
        && expect_patch
        && output.success
        && !output.timed_out
        && !output.idle_timed_out
        && !output.interrupted_by_supervisor
        && patch.trim().is_empty()
}

struct EmptyPatchFollowupRequest<'a> {
    root: &'a Path,
    mode: DelegationMode,
    task: &'a TaskSpec,
    task_path: &'a Path,
    out_dir: &'a Path,
    runner: &'a dyn OpenCodeRunner,
    require_local: bool,
    original_request: &'a OpenCodeRequest,
    output: &'a OpenCodeOutput,
}

fn run_empty_patch_followup(
    request: EmptyPatchFollowupRequest<'_>,
) -> Result<(OpenCodeOutput, PathBuf)> {
    let EmptyPatchFollowupRequest {
        root,
        mode,
        task,
        task_path,
        out_dir,
        runner,
        require_local,
        original_request,
        output,
    } = request;
    let resume_session_id = output.session_id.clone().ok_or_else(|| {
        anyhow!("cannot run empty-patch follow-up without an OpenCode session id")
    })?;
    let followup_dir = out_dir.join("empty-patch-followup");
    fs::create_dir_all(&followup_dir).with_context(|| {
        format!(
            "failed to create empty-patch follow-up dir {}",
            followup_dir.display()
        )
    })?;
    let followup_task = json!({
        "title": format!("Empty-patch follow-up: {}", task.title),
        "expect_patch": true,
        "instructions": "The previous local-worker run exited successfully, but Mixmod captured no repository diff.",
        "files": &task.files,
        "tests": &task.tests,
        "constraints": [
            "Do not restart broad exploration.",
            "Resolve the empty-patch mismatch compactly."
        ],
        "acceptance": [
            "Either make the intended edits, explain why no patch is needed, or explain the blocker."
        ],
        "context": {
            "source_task": task_path.to_string_lossy(),
            "empty_patch_followup": true
        }
    });
    let followup_task_path = followup_dir.join("task.json");
    write_pretty_json(
        &followup_task_path,
        &followup_task,
        "empty-patch follow-up task",
    )?;

    let instruction = build_empty_patch_followup_instruction(mode, task, task_path, &followup_dir);
    let instruction_path = followup_dir.join("opencode-instructions.md");
    atomic_write(&instruction_path, instruction.as_bytes())?;
    let followup_request = OpenCodeRequest {
        root: root.to_path_buf(),
        mode,
        task_path: followup_task_path,
        out_dir: followup_dir.clone(),
        instruction_path,
        instruction,
        session_id: original_request.session_id.clone(),
        resume_session_id: Some(resume_session_id),
        require_local,
    };
    let followup_output = runner.run(&followup_request)?;
    Ok((followup_output, followup_dir))
}

fn build_empty_patch_followup_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    _task_path: &Path,
    _out_dir: &Path,
) -> String {
    let files = if task.files.is_empty() {
        "- none specified".to_string()
    } else {
        task.files
            .iter()
            .map(|file| format!("- `{file}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tests = if task.tests.is_empty() {
        "- none specified".to_string()
    } else {
        task.tests
            .iter()
            .map(|test| format!("- `{test}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"# Empty-Patch Follow-Up

Mode: {mode}
Expected repository patch: yes

Mixmod-managed state lives under `.mixmod`. Do not read, grep, glob, edit, or rely on `.mixmod` files. The task content you need is embedded below.

The previous local-worker run exited successfully, but Mixmod captured no repository diff.

Confirm one of:
1. No patch is actually needed. Explain briefly why.
2. A patch is needed. Make the intended edits now, then finalize.
3. You are blocked. Explain the blocker briefly.

If you intended edits, do not finalize until they are made.

Relevant files:
{files}

Requested tests:
{tests}

Keep the final response compact.
"#,
        mode = mode,
        files = files,
        tests = tests,
    )
}

fn merge_opencode_outputs(mut first: OpenCodeOutput, second: OpenCodeOutput) -> OpenCodeOutput {
    first
        .command_for_metrics
        .push("<empty-patch-followup>".to_string());
    first
        .command_for_metrics
        .extend(second.command_for_metrics.clone());
    first
        .opencode_segments
        .extend(second.opencode_segments.clone());
    first.exit_status = second.exit_status;
    first.success = second.success;
    first
        .stdout
        .extend_from_slice(b"\n\n--- empty-patch follow-up stdout ---\n");
    first.stdout.extend_from_slice(&second.stdout);
    first
        .stderr
        .extend_from_slice(b"\n\n--- empty-patch follow-up stderr ---\n");
    first.stderr.extend_from_slice(&second.stderr);
    first.session_id = second.session_id.or(first.session_id);
    first.resume_session_id = second.resume_session_id.or(first.resume_session_id);
    first.worker_session_reused = first.worker_session_reused || second.worker_session_reused;
    first.interrupted_by_supervisor =
        first.interrupted_by_supervisor || second.interrupted_by_supervisor;
    first.supervisor_control_action = second
        .supervisor_control_action
        .or(first.supervisor_control_action);
    first
        .supervisor_control_events
        .extend(second.supervisor_control_events);
    first.timed_out = first.timed_out || second.timed_out;
    first.idle_timed_out = first.idle_timed_out || second.idle_timed_out;
    first.heartbeat_count += second.heartbeat_count;
    first.require_local = first.require_local || second.require_local;
    first.local_inference_verified = if first.require_local {
        first.local_inference_verified && second.local_inference_verified
    } else {
        first.local_inference_verified || second.local_inference_verified
    };
    first.gpu_activity_observed = first.gpu_activity_observed || second.gpu_activity_observed;
    first.backend_activity_observed =
        first.backend_activity_observed || second.backend_activity_observed;
    first
        .verification_notes
        .push("Empty-patch follow-up output was merged into this run.".to_string());
    first.verification_notes.extend(second.verification_notes);
    first
}

pub(crate) fn build_opencode_instruction(
    mode: DelegationMode,
    task: &TaskSpec,
    _task_path: &Path,
    _out_dir: &Path,
) -> Result<String> {
    let files = if task.files.is_empty() {
        "- none specified".to_string()
    } else {
        task.files
            .iter()
            .map(|file| format!("- `{file}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tests = if task.tests.is_empty() {
        "- none specified".to_string()
    } else {
        task.tests
            .iter()
            .map(|test| format!("- `{test}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let constraints = if task.constraints.is_empty() {
        "- Keep work bounded to the task.\n- Return concise findings and avoid long pasted logs."
            .to_string()
    } else {
        task.constraints
            .iter()
            .map(|constraint| format!("- {constraint}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let acceptance = if task.acceptance.is_empty() {
        "- State whether the task appears complete.".to_string()
    } else {
        task.acceptance
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(format!(
        r#"# Mixmod Local Worker Task

You are OpenCode acting as a local GPU-backed worker supervised by Codex through Mixmod.
Codex remains the final authority. Treat your own output as a draft artifact for review.

Mode: {mode}
Expected repository patch: {expected_patch}

Mixmod-managed state lives under `.mixmod`. Do not read, grep, glob, edit, or rely on `.mixmod` files. The task content you need is embedded below.

## Task

Title: {title}

{instructions}

## Relevant Files

{files}

## Requested Tests

{tests}

## Constraints

{constraints}

## Acceptance

{acceptance}

## Completion Self-Check

Before finalizing, do a completion self-check:
- Did you complete every edit you intended to make?
- If you intended checks or verification, did you complete them?
- If any intended edit or check remains incomplete, say exactly what remains incomplete.

Do not claim success if intended edits or intended checks are incomplete.

## Output Contract

Keep the final stdout response compact and include:
- Summary
- Changed files
- Tests run and results
- Risks or supervisor questions

Stop immediately after the requested tests pass. Do not keep exploring after a passing test run.
Do not paste long logs. Mixmod captures stdout, stderr, patch, metrics, and session artifacts on disk.
"#,
        mode = mode,
        expected_patch = match task.expect_patch {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unspecified",
        },
        title = task.title,
        instructions = task.instructions,
        files = files,
        tests = tests,
        constraints = constraints,
        acceptance = acceptance,
    ))
}

pub(crate) fn run_task_tests(
    root: &Path,
    logs_dir: &Path,
    tests: &[String],
) -> Result<(String, Vec<TestCommandResult>, TestArtifact)> {
    if tests.is_empty() {
        let artifact = TestArtifact {
            status: "not_requested".to_string(),
            requested: Vec::new(),
            observed: Vec::new(),
            notes: vec!["No test commands were listed in the task.".to_string()],
        };
        return Ok(("not_requested".to_string(), Vec::new(), artifact));
    }

    let mut results = Vec::new();
    for (index, test) in tests.iter().enumerate() {
        let label = index + 1;
        let stdout_path = logs_dir.join(format!("test-{label}.stdout.txt"));
        let stderr_path = logs_dir.join(format!("test-{label}.stderr.txt"));
        let start = Instant::now();
        let output = shell_command(test)
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        let wall_clock_ms = start.elapsed().as_millis();

        match output {
            Ok(output) => {
                atomic_write(&stdout_path, &output.stdout)?;
                atomic_write(&stderr_path, &output.stderr)?;
                results.push(TestCommandResult {
                    command: test.clone(),
                    status: if output.status.success() {
                        "passed".to_string()
                    } else {
                        "failed".to_string()
                    },
                    exit_status: output.status.code(),
                    wall_clock_ms,
                    stdout_bytes: output.stdout.len() as u64,
                    stderr_bytes: output.stderr.len() as u64,
                    stdout_log: display_path(root, &stdout_path),
                    stderr_log: display_path(root, &stderr_path),
                });
            }
            Err(error) => {
                let message = format!("failed to run test command `{test}`: {error}\n");
                atomic_write(&stdout_path, b"")?;
                atomic_write(&stderr_path, message.as_bytes())?;
                results.push(TestCommandResult {
                    command: test.clone(),
                    status: "failed".to_string(),
                    exit_status: None,
                    wall_clock_ms,
                    stdout_bytes: 0,
                    stderr_bytes: message.len() as u64,
                    stdout_log: display_path(root, &stdout_path),
                    stderr_log: display_path(root, &stderr_path),
                });
            }
        }
    }

    let status = if results.iter().all(|result| result.status == "passed") {
        "passed"
    } else {
        "failed"
    }
    .to_string();
    let artifact = TestArtifact {
        status: status.clone(),
        requested: tests.to_vec(),
        observed: results.clone(),
        notes: vec![
            "Mixmod ran these test commands after OpenCode completed and after changes.patch was captured.".to_string(),
            format!("Test logs live under {}.", display_path(root, logs_dir)),
        ],
    };
    Ok((status, results, artifact))
}

pub(crate) fn shell_command(command: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }

    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
}

pub(crate) fn build_run_summary(
    status: &str,
    mode: DelegationMode,
    output: &OpenCodeOutput,
    stats: &PatchStats,
    worktree_stats: &PatchStats,
    test_status: &str,
) -> String {
    match status {
        "success" => format!(
            "OpenCode completed {mode}; {} file(s) and {} line(s) changed; tests {test_status}.",
            stats.files.len(),
            stats.changed_line_count
        ),
        "needs_supervisor"
            if output.timed_out || output.idle_timed_out || output.interrupted_by_supervisor =>
        {
            let reason = if output.interrupted_by_supervisor {
                "supervisor control interrupt"
            } else if output.timed_out {
                "worker timeout"
            } else {
                "idle timeout"
            };
            format!(
                "OpenCode stopped for {mode} after {reason}; {} file(s) and {} line(s) were captured for supervisor recovery.",
                stats.files.len(),
                stats.changed_line_count
            )
        }
        "needs_supervisor" if !stats.files.is_empty() => format!(
            "OpenCode completed {mode} with {} file(s) and {} line(s) changed; tests {test_status}; supervisor review needed.",
            stats.files.len(),
            stats.changed_line_count
        ),
        "needs_supervisor" if !worktree_stats.files.is_empty() => format!(
            "OpenCode completed {mode} with no new delta, but current worktree patch has {} file(s) and {} line(s) changed; tests {test_status}; supervisor review needed.",
            worktree_stats.files.len(),
            worktree_stats.changed_line_count
        ),
        "needs_supervisor" => {
            format!(
                "OpenCode completed {mode} but no patch was captured; supervisor review needed."
            )
        }
        _ => format!(
            "OpenCode failed or could not be started for {mode}; exit status {:?}, stderr {} bytes.",
            output.exit_status,
            output.stderr.len()
        ),
    }
}

struct RunReportInput<'a> {
    status: &'a str,
    mode: DelegationMode,
    summary: &'a str,
    task: &'a TaskSpec,
    output: &'a OpenCodeOutput,
    stats: &'a PatchStats,
    worktree_stats: &'a PatchStats,
    test_status: &'a str,
    test_results: &'a [TestCommandResult],
    notes: &'a [String],
    root: &'a Path,
    out_dir: &'a Path,
}

fn build_run_report(input: RunReportInput<'_>) -> String {
    let RunReportInput {
        status,
        mode,
        summary,
        task,
        output,
        stats,
        worktree_stats,
        test_status,
        test_results,
        notes,
        root,
        out_dir,
    } = input;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let files = if stats.files.is_empty() {
        "- none captured".to_string()
    } else {
        stats
            .files
            .iter()
            .map(|file| format!("- `{file}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let requested_tests = if task.tests.is_empty() {
        "- none specified".to_string()
    } else {
        task.tests
            .iter()
            .map(|test| format!("- `{test}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let observed_tests = if test_results.is_empty() {
        "- none observed".to_string()
    } else {
        test_results
            .iter()
            .map(|test| {
                format!(
                    "- `{}`: {} (exit {:?}, {} ms)",
                    test.command, test.status, test.exit_status, test.wall_clock_ms
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let notes = notes
        .iter()
        .map(|note| format!("- {note}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"# Mixmod Run Report

## Summary

- Status: {status}
- Mode: {mode}
- Task: {task_title}
- Result: {summary}
- OpenCode exit status: {exit_status}
- OpenCode session label: {session_label}
- OpenCode session id: {session_id}
- OpenCode resumed session id: {resume_session_id}
- Worker session reused: {worker_session_reused}
- Interrupted by supervisor control: {interrupted_by_supervisor}
- Supervisor control action: {supervisor_control_action}
- OpenCode timed out: {timed_out}
- OpenCode idle timed out: {idle_timed_out}
- Heartbeats: {heartbeat_count}

## Changed Files

{files}

Changed lines: {changed_lines} ({added} added, {removed} removed)

## Current Worktree Patch

- Files: {worktree_files}
- Changed lines: {worktree_changed_lines} ({worktree_added} added, {worktree_removed} removed)
- Artifact: `{worktree_patch}`

## Tests

Structured test status: {test_status}

Requested tests:

{requested_tests}

Observed tests:

{observed_tests}

## OpenCode Stdout Excerpt

```text
{stdout_excerpt}
```

## OpenCode Stderr Excerpt

```text
{stderr_excerpt}
```

## Compact Artifact Paths

- `{receipt}`
- `{report}`
- `{worktree_patch}`
- `{patch}`
- `{tests}`
- `{metrics}`

Raw session and logs are available under `{out_dir}` when needed.
Heartbeat log: `{heartbeat}`

## Notes

{notes}
"#,
        status = status,
        mode = mode,
        task_title = task.title,
        summary = summary,
        exit_status = opencode_exit_status_label(output),
        session_label = output.session_label.as_deref().unwrap_or("unavailable"),
        session_id = output.session_id.as_deref().unwrap_or("unavailable"),
        resume_session_id = output.resume_session_id.as_deref().unwrap_or("none"),
        worker_session_reused = yes_no(output.worker_session_reused),
        interrupted_by_supervisor = yes_no(output.interrupted_by_supervisor),
        supervisor_control_action = output
            .supervisor_control_action
            .as_deref()
            .unwrap_or("none"),
        timed_out = yes_no(output.timed_out),
        idle_timed_out = yes_no(output.idle_timed_out),
        heartbeat_count = output.heartbeat_count,
        files = files,
        changed_lines = stats.changed_line_count,
        added = stats.added_lines,
        removed = stats.removed_lines,
        worktree_files = worktree_stats.files.len(),
        worktree_changed_lines = worktree_stats.changed_line_count,
        worktree_added = worktree_stats.added_lines,
        worktree_removed = worktree_stats.removed_lines,
        test_status = test_status,
        requested_tests = requested_tests,
        observed_tests = observed_tests,
        stdout_excerpt = truncate_for_report(&stdout, 4000),
        stderr_excerpt = truncate_for_report(&stderr, 4000),
        receipt = display_path(root, &out_dir.join("receipt.json")),
        report = display_path(root, &out_dir.join("report.md")),
        worktree_patch = display_path(root, &out_dir.join("worktree.patch")),
        patch = display_path(root, &out_dir.join("changes.patch")),
        tests = display_path(root, &out_dir.join("tests.json")),
        metrics = display_path(root, &out_dir.join("metrics.json")),
        heartbeat = display_path(root, &out_dir.join("logs/heartbeat.jsonl")),
        out_dir = display_path(root, out_dir),
        notes = notes,
    )
}

pub(crate) fn opencode_exit_status_label(output: &OpenCodeOutput) -> String {
    if let Some(code) = output.exit_status {
        return code.to_string();
    }
    if output.interrupted_by_supervisor {
        return "interrupted-by-supervisor".to_string();
    }
    if output.timed_out {
        return "worker-timeout".to_string();
    }
    if output.idle_timed_out {
        return "idle-timeout".to_string();
    }
    "spawn-failed".to_string()
}

fn build_session_jsonl(
    start: &DateTime<Utc>,
    end: &DateTime<Utc>,
    output: &OpenCodeOutput,
) -> Result<String> {
    let events = [
        json!({
            "event": "started",
            "timestamp": start.to_rfc3339(),
            "command": output.command_for_metrics,
            "session_label": output.session_label,
            "session_id": output.session_id,
            "resume_session_id": output.resume_session_id,
            "worker_session_reused": output.worker_session_reused,
            "interrupted_by_supervisor": output.interrupted_by_supervisor,
            "supervisor_control_action": output.supervisor_control_action,
            "opencode_segments": output.opencode_segments.clone(),
        }),
        json!({
            "event": "opencode_stdout",
            "timestamp": end.to_rfc3339(),
            "bytes": output.stdout.len(),
            "text": String::from_utf8_lossy(&output.stdout),
        }),
        json!({
            "event": "opencode_stderr",
            "timestamp": end.to_rfc3339(),
            "bytes": output.stderr.len(),
            "text": String::from_utf8_lossy(&output.stderr),
        }),
        json!({
            "event": "finished",
            "timestamp": end.to_rfc3339(),
            "exit_status": output.exit_status,
            "success": output.success,
            "timed_out": output.timed_out,
            "idle_timed_out": output.idle_timed_out,
            "heartbeat_count": output.heartbeat_count,
            "interrupted_by_supervisor": output.interrupted_by_supervisor,
            "supervisor_control_action": output.supervisor_control_action,
        }),
    ];
    let mut session = String::new();
    for event in events {
        session.push_str(
            &serde_json::to_string(&event).context("failed to serialize session JSONL event")?,
        );
        session.push('\n');
    }
    Ok(session)
}
