use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::TOOL_EVENTS_JSONL;
use crate::{
    DelegationMode, METRICS_JSON, MixmodConfig, REPORT_MD, ShellOpenCodeRunner, WORKTREE_PATCH,
    WorkerRunOptions, absolutize, append_jsonl, atomic_write, diff_without_unchanged_blocks,
    display_path, get_str, git_diff_with_untracked, load_config, patch_stats, read_json_file,
    run_mixmod_task_with_worker_options, shell_command, state_layout, write_pretty_json,
};

const CONFIG_SNAPSHOT_JSON: &str = "supervisor-tool-proxy-config.json";
const PAYLOAD_DIR: &str = "supervisor-tool-proxy-payloads";
const ASK_WORKER_TIMEOUT_SECONDS: u64 = 90;
const ASK_IDLE_TIMEOUT_SECONDS: u64 = 60;
const COMMAND_SUMMARY_WORKER_TIMEOUT_SECONDS: u64 = 45;
const COMMAND_SUMMARY_IDLE_TIMEOUT_SECONDS: u64 = 30;
const COMMAND_SUMMARY_BYTES: usize = 2_000;
const DIRECT_SUMMARY_MAX_EXACT_STDOUT_BYTES: u64 = 1_200;
const DIRECT_SUMMARY_MAX_EXACT_STDERR_BYTES: u64 = 600;
const ASK_SUMMARY_BYTES: usize = 3_000;
const COMMAND_TOOL_ARTIFACT_HINT: &str = "inspect artifacts_dir/{logs/command.stdout.txt,logs/command.stderr.txt,command-result.json,search-summary.json,logs/opencode.stdout.txt,logs/opencode.stderr.txt,tool-events.jsonl,worktree.patch} if answer is insufficient";
const ASK_TOOL_ARTIFACT_HINT: &str = "inspect artifacts_dir/{logs/opencode.stdout.txt,logs/opencode.stderr.txt,tool-events.jsonl,reasoning-trace.jsonl,worktree.patch} if answer is insufficient";
const SEARCH_SUMMARY_MAX_FILES: usize = 80;
const SEARCH_SUMMARY_HITS_PER_FILE: usize = 3;
const SEARCH_SUMMARY_HIT_CHARS: usize = 240;

/// Run a supervisor-requested prompt through the configured low-cost worker.
pub(crate) fn run_worker_ask_tool(root: &Path, prompt: &str) -> Result<()> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        anyhow::bail!("worker prompt must not be empty");
    }
    let payload = SupervisorToolProxyPayload::from_prompt(prompt, root);
    run_supervisor_tool_proxy_payload(&payload, root)
}

/// Run a supervisor-requested command through the configured low-cost worker.
pub(crate) fn run_worker_command_tool(
    root: &Path,
    command: &str,
    need: Option<&str>,
) -> Result<()> {
    let command = command.trim();
    if command.is_empty() {
        anyhow::bail!("worker command must not be empty");
    }
    let payload = SupervisorToolProxyPayload::from_command(command, need, root);
    run_supervisor_tool_proxy_payload(&payload, root)
}

/// Handle Codex `PreToolUse` input for the supervisor-scoped proxy hook.
pub(crate) fn codex_hook_pre_tool_use() -> Result<()> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("failed to read Codex hook input")?;
    let event: Value = serde_json::from_str(&input).context("failed to parse Codex hook JSON")?;
    let Some(command) = shell_command_from_pre_tool_use(&event) else {
        return Ok(());
    };
    if !should_proxy_bash_command(command) {
        return Ok(());
    }

    let code_home = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("CODEX_HOME was not set for supervisor tool proxy hook"))?;
    let config_path = env::var_os("MIXMOD_SUPERVISOR_TOOL_PROXY_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| code_home.join(CONFIG_SNAPSHOT_JSON));
    let payload = SupervisorToolProxyPayload::from_event(command, &event, config_path);
    let payload_path = payload_path(&code_home, &payload)?;
    write_pretty_json(&payload_path, &payload, "supervisor tool proxy payload")?;

    let exe = env::current_exe().context("failed to locate current mixmod executable")?;
    let replacement = format!(
        "{} codex-hook run-tool-proxy --payload {}",
        shell_quote_path(&exe),
        shell_quote_path(&payload_path)
    );
    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {
                "command": replacement,
                "cmd": replacement
            }
        }
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

/// Execute a worker-backed proxy payload and print a compact result for Codex.
pub(crate) fn run_supervisor_tool_proxy(payload_path: &Path, invocation_cwd: &Path) -> Result<()> {
    let payload: SupervisorToolProxyPayload = serde_json::from_value(read_json_file(payload_path)?)
        .with_context(|| format!("failed to parse payload {}", payload_path.display()))?;
    run_supervisor_tool_proxy_payload(&payload, invocation_cwd)
}

fn run_supervisor_tool_proxy_payload(
    payload: &SupervisorToolProxyPayload,
    invocation_cwd: &Path,
) -> Result<()> {
    let root = payload
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| invocation_cwd.to_path_buf())
        .canonicalize()
        .unwrap_or_else(|_| absolutize(invocation_cwd, Path::new(".")));
    let mut config = load_tool_proxy_config(&payload.config_path, &root)?;
    apply_tool_proxy_limits(payload.kind, &mut config);
    let runner = ShellOpenCodeRunner::new(config.clone());
    let out_dir = state_layout(&root)
        .project_dir()
        .join("supervisor-tool-proxy")
        .join(sanitize_id(payload.turn_id.as_deref().unwrap_or("turn")))
        .join(sanitize_id(
            payload.tool_use_id.as_deref().unwrap_or("tool"),
        ));
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;
    let task_path = out_dir.join("tool-task.json");
    let task = tool_proxy_task(&payload);
    write_pretty_json(&task_path, &task, "supervisor tool proxy task")?;

    if payload.kind == SupervisorToolProxyKind::Command {
        return run_supervisor_command_tool_proxy(payload, &root, &out_dir, config);
    }

    let receipt = run_mixmod_task_with_worker_options(
        &root,
        DelegationMode::Explore,
        &task_path,
        &out_dir,
        &runner,
        config.opencode.require_local,
        WorkerRunOptions {
            allow_auto_followups: false,
            ..WorkerRunOptions::default()
        },
    )?;

    let worker_text = extract_last_text(&out_dir.join("logs/opencode.stdout.txt"))
        .unwrap_or_else(|| "No compact worker text was captured.".to_string());
    let metrics = read_json_file(&out_dir.join(METRICS_JSON)).unwrap_or_else(|_| json!({}));
    print_ask_tool_result(
        payload,
        &root,
        &out_dir,
        &metrics,
        &receipt,
        &receipt.status,
        &worker_text,
    );
    Ok(())
}

fn run_supervisor_command_tool_proxy(
    payload: &SupervisorToolProxyPayload,
    root: &Path,
    out_dir: &Path,
    config: MixmodConfig,
) -> Result<()> {
    let before_diff = git_diff_with_untracked(root).unwrap_or_default();
    let command_result = run_command_directly(root, out_dir, payload.command.trim(), &config)?;
    let after_command_diff = git_diff_with_untracked(root).unwrap_or_default();
    atomic_write(&out_dir.join(WORKTREE_PATCH), after_command_diff.as_bytes())?;

    let command_delta = diff_without_unchanged_blocks(&after_command_diff, &before_diff);
    let command_delta_stats = patch_stats(&command_delta);
    let summary = deterministic_command_summary(payload, out_dir, &command_result)?
        .map(Ok)
        .unwrap_or_else(|| {
            run_command_summary_worker(
                payload,
                root,
                out_dir,
                &config,
                &command_result,
                &after_command_diff,
            )
        })?;
    let final_patch = git_diff_with_untracked(root).unwrap_or(after_command_diff);
    atomic_write(&out_dir.join(WORKTREE_PATCH), final_patch.as_bytes())?;

    let worker_status = tool_proxy_command_status(&command_result);
    print_command_tool_result(
        payload,
        root,
        out_dir,
        summary.metrics.as_ref().unwrap_or(&json!({})),
        &command_result,
        &command_delta_stats,
        summary.summary_delta_stats.as_ref(),
        &worker_status,
        &summary.text,
    );
    Ok(())
}

fn print_command_tool_result(
    payload: &SupervisorToolProxyPayload,
    root: &Path,
    out_dir: &Path,
    metrics: &Value,
    command_result: &CommandToolResult,
    command_delta_stats: &crate::PatchStats,
    summary_delta_stats: Option<&crate::PatchStats>,
    worker_status: &str,
    worker_text: &str,
) {
    print!(
        "{}",
        render_command_tool_result(
            payload,
            root,
            out_dir,
            metrics,
            command_result,
            command_delta_stats,
            summary_delta_stats,
            worker_status,
            worker_text,
        )
    );
}

fn render_command_tool_result(
    payload: &SupervisorToolProxyPayload,
    root: &Path,
    out_dir: &Path,
    metrics: &Value,
    command_result: &CommandToolResult,
    command_delta_stats: &crate::PatchStats,
    summary_delta_stats: Option<&crate::PatchStats>,
    worker_status: &str,
    worker_text: &str,
) -> String {
    let mut output = String::new();
    writeln!(output, "Mixmod command proxy result").expect("write to string");
    writeln!(output, "status: {worker_status}").expect("write to string");
    writeln!(output, "command: {}", payload.command.trim()).expect("write to string");
    if let Some(need) = payload.need_text() {
        writeln!(output, "need: {}", compact_text(need, 600)).expect("write to string");
    }
    if let Some(exit_status) = command_result.exit_status {
        writeln!(output, "command_exit_status: {exit_status}").expect("write to string");
    }
    writeln!(output, "command_timed_out: {}", command_result.timed_out).expect("write to string");
    writeln!(output, "command_stdout: logs/command.stdout.txt").expect("write to string");
    writeln!(output, "command_stderr: logs/command.stderr.txt").expect("write to string");
    writeln!(output, "command_result: command-result.json").expect("write to string");
    if out_dir.join("search-summary.json").exists() {
        writeln!(output, "search_summary: search-summary.json").expect("write to string");
    }
    if let Some(exit_status) = metrics.get("opencode_exit_status").and_then(Value::as_u64) {
        writeln!(output, "summary_worker_exit_status: {exit_status}").expect("write to string");
    }
    if command_delta_stats.changed_line_count > 0 || !command_delta_stats.files.is_empty() {
        writeln!(
            output,
            "command_side_effects: changed_files={} changed_lines={} files={}",
            command_delta_stats.files.len(),
            command_delta_stats.changed_line_count,
            compact_text(&command_delta_stats.files.join(", "), 800)
        )
        .expect("write to string");
    }
    if let Some(stats) = summary_delta_stats
        && (stats.changed_line_count > 0 || !stats.files.is_empty())
    {
        writeln!(
            output,
            "summary_worker_side_effects: changed_files={} changed_lines={} files={}",
            stats.files.len(),
            stats.changed_line_count,
            compact_text(&stats.files.join(", "), 800)
        )
        .expect("write to string");
    }
    writeln!(output, "artifacts_dir: {}", display_path(root, out_dir)).expect("write to string");
    writeln!(output, "artifacts: {COMMAND_TOOL_ARTIFACT_HINT}").expect("write to string");
    writeln!(output, "answer:").expect("write to string");
    writeln!(output, "{}", command_answer_text(worker_text)).expect("write to string");
    output
}

fn command_answer_text(worker_text: &str) -> String {
    if worker_text.len() <= COMMAND_SUMMARY_BYTES {
        return worker_text.trim().to_string();
    }
    format!(
        "summary_too_long: local worker answer exceeded {COMMAND_SUMMARY_BYTES} bytes; inspect artifacts_dir/logs/opencode.stdout.txt or command artifacts for exact evidence"
    )
}

fn print_ask_tool_result(
    payload: &SupervisorToolProxyPayload,
    root: &Path,
    out_dir: &Path,
    metrics: &Value,
    receipt: &crate::Receipt,
    worker_status: &str,
    worker_text: &str,
) {
    print!(
        "{}",
        render_ask_tool_result(
            payload,
            root,
            out_dir,
            metrics,
            receipt,
            worker_status,
            worker_text,
        )
    );
}

fn render_ask_tool_result(
    payload: &SupervisorToolProxyPayload,
    root: &Path,
    out_dir: &Path,
    metrics: &Value,
    receipt: &crate::Receipt,
    worker_status: &str,
    worker_text: &str,
) -> String {
    let mut output = String::new();
    writeln!(output, "Mixmod ask proxy result").expect("write to string");
    writeln!(
        output,
        "request: {}",
        compact_text(payload.request_text(), 1_000)
    )
    .expect("write to string");
    writeln!(output, "status: {worker_status}").expect("write to string");
    if let Some(exit_status) = metrics.get("opencode_exit_status").and_then(Value::as_u64) {
        writeln!(output, "worker_exit_status: {exit_status}").expect("write to string");
    }
    if let Some(notice) = tool_proxy_side_effect_notice(receipt, metrics) {
        writeln!(output, "{notice}").expect("write to string");
        writeln!(output, "side_effect_patch_artifact: {}", WORKTREE_PATCH)
            .expect("write to string");
    }
    writeln!(output, "artifacts_dir: {}", display_path(root, out_dir)).expect("write to string");
    writeln!(output, "artifacts: {ASK_TOOL_ARTIFACT_HINT}").expect("write to string");
    writeln!(output, "answer:").expect("write to string");
    writeln!(output, "{}", compact_text(worker_text, ASK_SUMMARY_BYTES)).expect("write to string");
    output
}

#[derive(Clone, Debug)]
struct CommandSummaryResult {
    text: String,
    metrics: Option<Value>,
    summary_delta_stats: Option<crate::PatchStats>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CommandToolResult {
    exit_status: Option<i64>,
    timed_out: bool,
    duration_ms: u128,
    stdout_bytes: u64,
    stderr_bytes: u64,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReadableCommandArtifacts {
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    result_path: PathBuf,
    search_summary_path: Option<PathBuf>,
}

fn run_command_directly(
    root: &Path,
    out_dir: &Path,
    command: &str,
    config: &MixmodConfig,
) -> Result<CommandToolResult> {
    let logs_dir = out_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;
    let stdout_path = logs_dir.join("command.stdout.txt");
    let stderr_path = logs_dir.join("command.stderr.txt");
    let timeout_seconds = command_timeout_seconds(config);
    let started_at = Utc::now();
    let start = Instant::now();
    let output = deterministic_shell_command(command, timeout_seconds)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to run command through Mixmod: {command}"))?;
    let duration_ms = start.elapsed().as_millis();
    atomic_write(&stdout_path, &output.stdout)?;
    atomic_write(&stderr_path, &output.stderr)?;

    let exit_status = output.status.code().map(i64::from);
    let timed_out = timeout_seconds > 0 && exit_status == Some(124);
    let result = CommandToolResult {
        exit_status,
        timed_out,
        duration_ms,
        stdout_bytes: output.stdout.len() as u64,
        stderr_bytes: output.stderr.len() as u64,
        stdout_path: stdout_path.clone(),
        stderr_path: stderr_path.clone(),
    };
    let result_json = json!({
        "kind": "mixmod-command-result",
        "command": command,
        "started_at": started_at.to_rfc3339(),
        "finished_at": Utc::now().to_rfc3339(),
        "duration_ms": duration_ms,
        "exit_status": exit_status,
        "timed_out": timed_out,
        "timeout_seconds": timeout_seconds,
        "stdout_bytes": result.stdout_bytes,
        "stderr_bytes": result.stderr_bytes,
        "stdout_path": "logs/command.stdout.txt",
        "stderr_path": "logs/command.stderr.txt",
    });
    write_pretty_json(
        &out_dir.join("command-result.json"),
        &result_json,
        "command result",
    )?;
    append_jsonl(
        &out_dir.join(TOOL_EVENTS_JSONL),
        &json!({
            "type": "tool_use",
            "timestamp": Utc::now().to_rfc3339(),
            "part": {
                "tool": "bash",
                "state": {
                    "status": "completed",
                    "input": {"command": command},
                    "metadata": {
                        "exit": exit_status,
                        "timed_out": timed_out,
                        "duration_ms": duration_ms,
                        "stdout_path": "logs/command.stdout.txt",
                        "stderr_path": "logs/command.stderr.txt",
                        "stdout_bytes": result.stdout_bytes,
                        "stderr_bytes": result.stderr_bytes
                    }
                }
            }
        }),
    )?;
    Ok(result)
}

fn deterministic_shell_command(command: &str, timeout_seconds: u64) -> std::process::Command {
    #[cfg(unix)]
    {
        if timeout_seconds > 0 {
            let mut cmd = std::process::Command::new("timeout");
            cmd.arg(format!("{timeout_seconds}s"))
                .arg("sh")
                .arg("-c")
                .arg(command);
            return cmd;
        }
    }
    shell_command(command)
}

fn command_timeout_seconds(config: &MixmodConfig) -> u64 {
    env::var("MIXMOD_TOOL_COMMAND_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(config.opencode.worker_timeout_seconds)
}

fn run_command_summary_worker(
    payload: &SupervisorToolProxyPayload,
    root: &Path,
    out_dir: &Path,
    config: &MixmodConfig,
    command_result: &CommandToolResult,
    before_summary_diff: &str,
) -> Result<CommandSummaryResult> {
    let summary_task_path = out_dir.join("command-summary-task.json");
    let summary_task = command_summary_task(payload, root, out_dir, command_result)?;
    write_pretty_json(&summary_task_path, &summary_task, "command summary task")?;

    let mut summary_config = config.clone();
    apply_command_summary_limits(&mut summary_config);
    let runner = ShellOpenCodeRunner::new(summary_config.clone());
    let _receipt = run_mixmod_task_with_worker_options(
        root,
        DelegationMode::Explore,
        &summary_task_path,
        out_dir,
        &runner,
        summary_config.opencode.require_local,
        WorkerRunOptions {
            allow_auto_followups: false,
            ..WorkerRunOptions::default()
        },
    );
    let after_summary_diff = git_diff_with_untracked(root).unwrap_or_default();
    let summary_delta = diff_without_unchanged_blocks(&after_summary_diff, before_summary_diff);
    let summary_delta_stats = patch_stats(&summary_delta);
    let metrics = read_json_file(&out_dir.join(METRICS_JSON)).ok();
    let fallback = fallback_command_summary(payload, out_dir, command_result);
    let text = extract_last_text(&out_dir.join("logs/opencode.stdout.txt")).unwrap_or(fallback);

    Ok(CommandSummaryResult {
        text,
        metrics,
        summary_delta_stats: Some(summary_delta_stats),
    })
}

fn deterministic_command_summary(
    payload: &SupervisorToolProxyPayload,
    out_dir: &Path,
    command_result: &CommandToolResult,
) -> Result<Option<CommandSummaryResult>> {
    let stdout = read_text_lossy(&command_result.stdout_path);
    let stderr = read_text_lossy(&command_result.stderr_path);
    let search_summary = write_search_summary_artifact(payload, out_dir, &stdout)?;
    let Some(summary_kind) = direct_summary_kind(
        payload.command.trim(),
        command_result,
        search_summary.as_ref(),
    ) else {
        return Ok(None);
    };
    let text = deterministic_command_summary_text(
        payload,
        command_result,
        &stdout,
        &stderr,
        search_summary.as_ref(),
        summary_kind,
    );
    let metrics = json!({
        "kind": "mixmod-command-proxy",
        "status": tool_proxy_command_status(command_result),
        "summary_source": "mixmod_deterministic",
        "summary_kind": summary_kind.as_str(),
        "opencode_call": false,
        "local_inference_verified": false,
        "command_exit_status": command_result.exit_status,
        "command_timed_out": command_result.timed_out,
        "duration_ms": command_result.duration_ms,
        "stdout_bytes": command_result.stdout_bytes,
        "stderr_bytes": command_result.stderr_bytes,
        "search_summary_artifact": search_summary.as_ref().map(|_| "search-summary.json"),
    });
    write_pretty_json(
        &out_dir.join(METRICS_JSON),
        &metrics,
        "deterministic command proxy metrics",
    )?;
    atomic_write(
        &out_dir.join(REPORT_MD),
        deterministic_command_report(payload, command_result, summary_kind, &text).as_bytes(),
    )?;
    Ok(Some(CommandSummaryResult {
        text,
        metrics: Some(metrics),
        summary_delta_stats: Some(crate::PatchStats::default()),
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectCommandSummaryKind {
    DiffCheck,
    DiffStat,
    GitStatus,
    Search,
    SmallExactOutput,
    TestPass,
}

impl DirectCommandSummaryKind {
    fn as_str(self) -> &'static str {
        match self {
            DirectCommandSummaryKind::DiffCheck => "git_diff_check",
            DirectCommandSummaryKind::DiffStat => "git_diff_stat",
            DirectCommandSummaryKind::GitStatus => "git_status",
            DirectCommandSummaryKind::Search => "search_summary",
            DirectCommandSummaryKind::SmallExactOutput => "small_exact_output",
            DirectCommandSummaryKind::TestPass => "test_pass",
        }
    }
}

fn direct_summary_kind(
    command: &str,
    command_result: &CommandToolResult,
    search_summary: Option<&Value>,
) -> Option<DirectCommandSummaryKind> {
    if search_summary.is_some() {
        return Some(DirectCommandSummaryKind::Search);
    }
    if command_segments(command).any(is_git_status_command) {
        return Some(DirectCommandSummaryKind::GitStatus);
    }
    if command_segments(command).any(is_git_diff_check_command) {
        return Some(DirectCommandSummaryKind::DiffCheck);
    }
    if command_segments(command).any(is_git_diff_stat_command) {
        return Some(DirectCommandSummaryKind::DiffStat);
    }
    if command_result.exit_status == Some(0)
        && command_segments(command).any(is_test_or_build_command)
    {
        return Some(DirectCommandSummaryKind::TestPass);
    }
    if command_result.stdout_bytes <= DIRECT_SUMMARY_MAX_EXACT_STDOUT_BYTES
        && command_result.stderr_bytes <= DIRECT_SUMMARY_MAX_EXACT_STDERR_BYTES
    {
        return Some(DirectCommandSummaryKind::SmallExactOutput);
    }
    None
}

fn deterministic_command_summary_text(
    payload: &SupervisorToolProxyPayload,
    command_result: &CommandToolResult,
    stdout: &str,
    stderr: &str,
    search_summary: Option<&Value>,
    summary_kind: DirectCommandSummaryKind,
) -> String {
    let mut summary = String::new();
    writeln!(summary, "summary_source: mixmod_deterministic").expect("write to string");
    writeln!(summary, "summary_kind: {}", summary_kind.as_str()).expect("write to string");
    writeln!(
        summary,
        "exit_status: {}",
        command_result
            .exit_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    )
    .expect("write to string");
    writeln!(summary, "timed_out: {}", command_result.timed_out).expect("write to string");
    if let Some(need) = payload.need_text() {
        writeln!(summary, "need: {}", compact_text(need, 300)).expect("write to string");
    }
    match summary_kind {
        DirectCommandSummaryKind::Search => {
            if let Some(search_summary) = search_summary {
                append_search_summary_text(&mut summary, search_summary);
            }
            writeln!(summary, "search_summary: search-summary.json").expect("write to string");
            writeln!(summary, "stdout_artifact: logs/command.stdout.txt").expect("write to string");
        }
        DirectCommandSummaryKind::TestPass => {
            writeln!(summary, "pass: command exited 0").expect("write to string");
            append_last_nonempty_line(&mut summary, "stdout_last_line", stdout);
            append_last_nonempty_line(&mut summary, "stderr_last_line", stderr);
        }
        DirectCommandSummaryKind::GitStatus
        | DirectCommandSummaryKind::DiffCheck
        | DirectCommandSummaryKind::DiffStat
        | DirectCommandSummaryKind::SmallExactOutput => {
            append_exact_output(&mut summary, "stdout", stdout, command_result.stdout_bytes);
            append_exact_output(&mut summary, "stderr", stderr, command_result.stderr_bytes);
        }
    }
    writeln!(summary, "stdout_bytes: {}", command_result.stdout_bytes).expect("write to string");
    writeln!(summary, "stderr_bytes: {}", command_result.stderr_bytes).expect("write to string");
    writeln!(summary, "stdout_artifact: logs/command.stdout.txt").expect("write to string");
    writeln!(summary, "stderr_artifact: logs/command.stderr.txt").expect("write to string");
    compact_text(&summary, COMMAND_SUMMARY_BYTES)
}

fn append_search_summary_text(output: &mut String, summary: &Value) {
    if let Some(kind) = summary.get("kind").and_then(Value::as_str) {
        writeln!(output, "search_kind: {kind}").expect("write to string");
    }
    for key in [
        "total_output_lines",
        "returned_files",
        "returned_paths",
        "files_truncated",
        "paths_truncated",
        "omitted_hits_after_file_limit",
    ] {
        if let Some(value) = summary.get(key) {
            writeln!(output, "{key}: {value}").expect("write to string");
        }
    }
    if let Some(files) = summary.get("files").and_then(Value::as_array) {
        for file in files.iter().take(8) {
            let path = file
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let matches = file.get("match_count").and_then(Value::as_u64).unwrap_or(0);
            let first_hit = file
                .get("hits")
                .and_then(Value::as_array)
                .and_then(|hits| hits.first())
                .and_then(Value::as_str)
                .unwrap_or("");
            writeln!(
                output,
                "- {path}: matches={matches} first_hit={}",
                compact_chars(first_hit, 160)
            )
            .expect("write to string");
        }
    }
    if let Some(paths) = summary.get("paths").and_then(Value::as_array) {
        for path in paths.iter().take(12).filter_map(Value::as_str) {
            writeln!(output, "- {path}").expect("write to string");
        }
    }
}

fn append_exact_output(output: &mut String, label: &str, text: &str, bytes: u64) {
    let text = text.trim_end();
    if text.is_empty() {
        writeln!(output, "{label}: <empty>").expect("write to string");
        return;
    }
    let limit = if label == "stdout" {
        DIRECT_SUMMARY_MAX_EXACT_STDOUT_BYTES
    } else {
        DIRECT_SUMMARY_MAX_EXACT_STDERR_BYTES
    };
    if bytes <= limit {
        writeln!(output, "{label}:").expect("write to string");
        writeln!(output, "```text").expect("write to string");
        writeln!(output, "{text}").expect("write to string");
        writeln!(output, "```").expect("write to string");
    } else {
        writeln!(output, "{label}: <see {label}_artifact; {bytes} bytes>")
            .expect("write to string");
    }
}

fn append_last_nonempty_line(output: &mut String, label: &str, text: &str) {
    if let Some(line) = text.lines().rev().find(|line| !line.trim().is_empty()) {
        writeln!(output, "{label}: {}", compact_chars(line, 240)).expect("write to string");
    }
}

fn deterministic_command_report(
    payload: &SupervisorToolProxyPayload,
    command_result: &CommandToolResult,
    summary_kind: DirectCommandSummaryKind,
    text: &str,
) -> String {
    format!(
        "# Mixmod Command Proxy Report\n\n\
         - Status: {}\n\
         - Summary source: mixmod_deterministic\n\
         - Summary kind: {}\n\
         - Command: `{}`\n\
         - Exit status: {}\n\
         - Timed out: {}\n\
         - Stdout bytes: {}\n\
         - Stderr bytes: {}\n\n\
         ## Answer\n\n{}\n",
        tool_proxy_command_status(command_result),
        summary_kind.as_str(),
        payload.command.trim().replace('`', "\\`"),
        command_result
            .exit_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        command_result.timed_out,
        command_result.stdout_bytes,
        command_result.stderr_bytes,
        text
    )
}

fn command_summary_task(
    payload: &SupervisorToolProxyPayload,
    root: &Path,
    out_dir: &Path,
    command_result: &CommandToolResult,
) -> Result<Value> {
    let stdout = read_text_lossy(&command_result.stdout_path);
    let search_summary = write_search_summary_artifact(payload, out_dir, &stdout)?;
    let readable_artifacts =
        prepare_readable_command_artifacts(root, out_dir, search_summary.is_some())?;
    let artifact_section = command_artifact_section(&readable_artifacts);
    let need = payload
        .need_text()
        .unwrap_or("Return the useful command result compactly.");
    Ok(json!({
        "title": format!("Summarize command result: {}", payload.command.trim()),
        "instructions": format!(
            "A GPT supervisor requested a deterministic shell command. Mixmod already ran the command; do not run shell commands, do not inspect the repository, do not edit files, and do not commit. Read only the listed command artifact files when needed, then summarize the captured command result for the supervisor information need.\n\nSupervisor information need:\n{need}\n\nCommand:\n```bash\n{command}\n```\n\nExit status: {exit_status}\nTimed out: {timed_out}\nDuration ms: {duration_ms}\nStdout bytes: {stdout_bytes}\nStderr bytes: {stderr_bytes}\n\n{artifact_section}\n\nReturn compact semantic evidence: exit status, pass/fail when applicable, failing test names, first relevant traceback/assertion lines, matched files/symbols, or notable diff hunks. Do not copy raw stdout/stderr into the answer and do not return truncated output. If the output is too large or cannot be summarized confidently, say that and name the artifact path the supervisor should inspect.",
            command = payload.command.trim(),
            exit_status = command_result
                .exit_status
                .map(|status| status.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            timed_out = command_result.timed_out,
            duration_ms = command_result.duration_ms,
            stdout_bytes = command_result.stdout_bytes,
            stderr_bytes = command_result.stderr_bytes,
            artifact_section = artifact_section,
        ),
        "expect_patch": false,
        "tests": [],
        "constraints": [
            "Do not run shell commands.",
            "Do not inspect repository files.",
            "Do not edit files.",
            "Do not commit changes.",
            "Read only the listed command artifact files when needed.",
            "Do not copy raw stdout or stderr into the answer.",
            "Do not return truncated command output.",
            "Optimize the final answer for the supervisor information need."
        ],
        "context": {
            "worker_role": "command_output_summarizer",
            "expect_patch": false,
            "delegated_from": "mixmod_cli_tool",
            "original_command": payload.command.trim(),
            "supervisor_need": need,
            "command_exit_status": command_result.exit_status,
            "command_timed_out": command_result.timed_out,
            "stdout_artifact": readable_artifacts.stdout_path,
            "stderr_artifact": readable_artifacts.stderr_path,
            "command_result_artifact": readable_artifacts.result_path,
            "search_summary_artifact": readable_artifacts.search_summary_path
        }
    }))
}

fn read_text_lossy(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| {
        String::from_utf8_lossy(&fs::read(path).unwrap_or_default()).to_string()
    })
}

fn write_search_summary_artifact(
    payload: &SupervisorToolProxyPayload,
    out_dir: &Path,
    stdout: &str,
) -> Result<Option<Value>> {
    let search_summary = search_output_summary(payload.command.trim(), stdout);
    if let Some(summary) = &search_summary {
        write_pretty_json(
            &out_dir.join("search-summary.json"),
            summary,
            "command search summary",
        )?;
    }
    Ok(search_summary)
}

fn prepare_readable_command_artifacts(
    root: &Path,
    out_dir: &Path,
    has_search_summary: bool,
) -> Result<ReadableCommandArtifacts> {
    let readable_dir = state_layout(root)
        .project_dir()
        .join("xdg-data")
        .join("opencode")
        .join("tool-output");
    fs::create_dir_all(&readable_dir)
        .with_context(|| format!("failed to create {}", readable_dir.display()))?;
    let prefix = sanitize_id(
        out_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("command"),
    );
    let stdout_path = readable_dir.join(format!("{prefix}-command.stdout.txt"));
    let stderr_path = readable_dir.join(format!("{prefix}-command.stderr.txt"));
    let result_path = readable_dir.join(format!("{prefix}-command-result.json"));
    fs::copy(out_dir.join("logs/command.stdout.txt"), &stdout_path)
        .with_context(|| format!("failed to mirror {}", stdout_path.display()))?;
    fs::copy(out_dir.join("logs/command.stderr.txt"), &stderr_path)
        .with_context(|| format!("failed to mirror {}", stderr_path.display()))?;
    fs::copy(out_dir.join("command-result.json"), &result_path)
        .with_context(|| format!("failed to mirror {}", result_path.display()))?;
    let search_summary_path = if has_search_summary {
        let path = readable_dir.join(format!("{prefix}-search-summary.json"));
        fs::copy(out_dir.join("search-summary.json"), &path)
            .with_context(|| format!("failed to mirror {}", path.display()))?;
        Some(path)
    } else {
        None
    };
    Ok(ReadableCommandArtifacts {
        stdout_path,
        stderr_path,
        result_path,
        search_summary_path,
    })
}

fn command_artifact_section(artifacts: &ReadableCommandArtifacts) -> String {
    let mut section = String::new();
    writeln!(
        section,
        "Stdout artifact: {}",
        artifacts.stdout_path.display()
    )
    .expect("write to string");
    writeln!(
        section,
        "Stderr artifact: {}",
        artifacts.stderr_path.display()
    )
    .expect("write to string");
    writeln!(
        section,
        "Command result artifact: {}",
        artifacts.result_path.display()
    )
    .expect("write to string");
    if let Some(path) = &artifacts.search_summary_path {
        writeln!(section, "Search summary artifact: {}", path.display()).expect("write to string");
    }
    section
}

fn search_output_summary(command: &str, stdout: &str) -> Option<Value> {
    let kind = search_command_kind(command)?;
    match kind {
        SearchCommandKind::PathList => Some(path_search_summary(stdout)),
        SearchCommandKind::LineSearch => Some(line_search_summary(stdout)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchCommandKind {
    LineSearch,
    PathList,
}

fn search_command_kind(command: &str) -> Option<SearchCommandKind> {
    for segment in command_segments(command) {
        if segment == "find" || segment.starts_with("find ") {
            return Some(SearchCommandKind::PathList);
        }
        if segment == "rg"
            || segment.starts_with("rg ")
            || segment == "grep"
            || segment.starts_with("grep ")
            || segment == "git grep"
            || segment.starts_with("git grep ")
        {
            return Some(SearchCommandKind::LineSearch);
        }
    }
    None
}

fn path_search_summary(stdout: &str) -> Value {
    let mut total_lines = 0usize;
    let mut paths = Vec::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        total_lines += 1;
        if paths.len() < SEARCH_SUMMARY_MAX_FILES && !paths.iter().any(|path| path == line) {
            paths.push(line.to_string());
        }
    }
    json!({
        "kind": "path_search",
        "total_output_lines": total_lines,
        "returned_paths": paths.len(),
        "paths_truncated": total_lines > paths.len(),
        "paths": paths,
    })
}

fn line_search_summary(stdout: &str) -> Value {
    let mut total_lines = 0usize;
    let mut files = Vec::<SearchFileSummary>::new();
    let mut overflow_hits = 0usize;

    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        total_lines += 1;
        let (path, hit) = parse_search_hit(line);
        if let Some(index) = files.iter().position(|file| file.path == path) {
            files[index].match_count += 1;
            if files[index].hits.len() < SEARCH_SUMMARY_HITS_PER_FILE {
                files[index].hits.push(hit);
            }
        } else if files.len() < SEARCH_SUMMARY_MAX_FILES {
            files.push(SearchFileSummary {
                path,
                match_count: 1,
                hits: vec![hit],
            });
        } else {
            overflow_hits += 1;
        }
    }

    json!({
        "kind": "line_search",
        "total_output_lines": total_lines,
        "returned_files": files.len(),
        "files_truncated": overflow_hits > 0,
        "omitted_hits_after_file_limit": overflow_hits,
        "files": files,
    })
}

#[derive(Debug, Serialize)]
struct SearchFileSummary {
    path: String,
    match_count: usize,
    hits: Vec<String>,
}

fn parse_search_hit(line: &str) -> (String, String) {
    let mut parts = line.splitn(3, ':');
    let first = parts.next().unwrap_or("").trim();
    let second = parts.next().unwrap_or("").trim();
    let third = parts.next().unwrap_or("").trim();
    if !first.is_empty() && !second.is_empty() && second.chars().all(|ch| ch.is_ascii_digit()) {
        return (
            first.to_string(),
            format!(
                "{}: {}",
                second,
                compact_chars(third, SEARCH_SUMMARY_HIT_CHARS)
            ),
        );
    }
    if !first.is_empty() && !second.is_empty() {
        return (
            first.to_string(),
            compact_chars(&line[first.len() + 1..], SEARCH_SUMMARY_HIT_CHARS),
        );
    }
    (
        "<stdout>".to_string(),
        compact_chars(line, SEARCH_SUMMARY_HIT_CHARS),
    )
}

fn compact_chars(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut value: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    value.push_str("...");
    value
}

fn fallback_command_summary(
    payload: &SupervisorToolProxyPayload,
    out_dir: &Path,
    command_result: &CommandToolResult,
) -> String {
    let mut summary = String::new();
    writeln!(
        summary,
        "exit_status: {}",
        command_result
            .exit_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    )
    .expect("write to string");
    writeln!(summary, "timed_out: {}", command_result.timed_out).expect("write to string");
    if let Some(need) = payload.need_text() {
        writeln!(summary, "need: {}", compact_text(need, 400)).expect("write to string");
    }
    writeln!(
        summary,
        "summary_unavailable: local worker did not return a command summary"
    )
    .expect("write to string");
    writeln!(summary, "stdout_artifact: logs/command.stdout.txt").expect("write to string");
    writeln!(summary, "stderr_artifact: logs/command.stderr.txt").expect("write to string");
    writeln!(summary, "command_result: command-result.json").expect("write to string");
    writeln!(summary, "stdout_bytes: {}", command_result.stdout_bytes).expect("write to string");
    writeln!(summary, "stderr_bytes: {}", command_result.stderr_bytes).expect("write to string");
    if out_dir.join("search-summary.json").exists() {
        writeln!(summary, "search_summary: search-summary.json").expect("write to string");
    }
    summary
}

#[cfg(test)]
fn command_tool_result(out_dir: &Path, command: &str) -> Option<CommandToolResult> {
    let events = std::fs::read_to_string(out_dir.join(TOOL_EVENTS_JSONL)).ok()?;
    let mut result = None;
    for line in events.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(part) = event.get("part") else {
            continue;
        };
        if get_str(part, "tool") != Some("bash") {
            continue;
        }
        let Some(state) = part.get("state") else {
            continue;
        };
        let Some(actual_command) = state
            .get("input")
            .and_then(|input| get_str(input, "command"))
        else {
            continue;
        };
        if actual_command.trim() != command {
            continue;
        }
        let exit_status = state
            .get("metadata")
            .and_then(|metadata| metadata.get("exit"))
            .and_then(Value::as_i64);
        result = Some(CommandToolResult {
            exit_status,
            timed_out: false,
            duration_ms: 0,
            stdout_bytes: 0,
            stderr_bytes: 0,
            stdout_path: out_dir.join("logs/command.stdout.txt"),
            stderr_path: out_dir.join("logs/command.stderr.txt"),
        });
    }
    result
}

fn tool_proxy_command_status(command_result: &CommandToolResult) -> String {
    if command_result.timed_out {
        return "command_timed_out".to_string();
    }
    if matches!(command_result.exit_status, Some(status) if status != 0) {
        return "command_failed".to_string();
    }
    "success".to_string()
}

fn tool_proxy_side_effect_notice(receipt: &crate::Receipt, metrics: &Value) -> Option<String> {
    let changed_files = metrics
        .get("changed_file_count")
        .and_then(Value::as_u64)
        .unwrap_or(receipt.changed_files.len() as u64);
    let changed_lines = metrics
        .get("changed_line_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if changed_files == 0 && changed_lines == 0 {
        return None;
    }

    let mut notice =
        format!("worker_side_effects: changed_files={changed_files} changed_lines={changed_lines}");
    if !receipt.changed_files.is_empty() {
        let files = receipt.changed_files.join(", ");
        notice.push_str(" files=");
        notice.push_str(&compact_text(&files, 800));
    }
    Some(notice)
}

fn apply_tool_proxy_limits(kind: SupervisorToolProxyKind, config: &mut MixmodConfig) {
    if kind != SupervisorToolProxyKind::Ask {
        return;
    }
    config.opencode.worker_timeout_seconds = bounded_timeout(
        config.opencode.worker_timeout_seconds,
        ASK_WORKER_TIMEOUT_SECONDS,
    );
    config.opencode.idle_timeout_seconds = bounded_timeout(
        config.opencode.idle_timeout_seconds,
        ASK_IDLE_TIMEOUT_SECONDS,
    );
}

fn apply_command_summary_limits(config: &mut MixmodConfig) {
    config.opencode.worker_timeout_seconds = bounded_timeout(
        config.opencode.worker_timeout_seconds,
        COMMAND_SUMMARY_WORKER_TIMEOUT_SECONDS,
    );
    config.opencode.idle_timeout_seconds = bounded_timeout(
        config.opencode.idle_timeout_seconds,
        COMMAND_SUMMARY_IDLE_TIMEOUT_SECONDS,
    );
}

fn bounded_timeout(current: u64, limit: u64) -> u64 {
    if current == 0 {
        limit
    } else {
        current.min(limit)
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct SupervisorToolProxyPayload {
    #[serde(default)]
    kind: SupervisorToolProxyKind,
    command: String,
    #[serde(default)]
    need: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    cwd: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    tool_use_id: Option<String>,
    model: Option<String>,
    config_path: PathBuf,
    created_at: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum SupervisorToolProxyKind {
    #[default]
    Command,
    Ask,
}

impl SupervisorToolProxyPayload {
    fn from_command(command: &str, need: Option<&str>, root: &Path) -> Self {
        Self {
            kind: SupervisorToolProxyKind::Command,
            command: command.to_string(),
            need: need
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            prompt: None,
            cwd: Some(root.to_string_lossy().to_string()),
            session_id: None,
            turn_id: Some("cli".to_string()),
            tool_use_id: Some(format!("tool-{}", Utc::now().format("%Y%m%dT%H%M%S%.3fZ"))),
            model: None,
            config_path: state_layout(root).config(),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn from_prompt(prompt: &str, root: &Path) -> Self {
        Self {
            kind: SupervisorToolProxyKind::Ask,
            command: String::new(),
            need: None,
            prompt: Some(prompt.to_string()),
            cwd: Some(root.to_string_lossy().to_string()),
            session_id: None,
            turn_id: Some("cli".to_string()),
            tool_use_id: Some(format!("ask-{}", Utc::now().format("%Y%m%dT%H%M%S%.3fZ"))),
            model: None,
            config_path: state_layout(root).config(),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn from_event(command: &str, event: &Value, config_path: PathBuf) -> Self {
        Self {
            kind: SupervisorToolProxyKind::Command,
            command: command.to_string(),
            need: None,
            prompt: None,
            cwd: get_str(event, "cwd").map(ToOwned::to_owned),
            session_id: get_str(event, "session_id").map(ToOwned::to_owned),
            turn_id: get_str(event, "turn_id").map(ToOwned::to_owned),
            tool_use_id: get_str(event, "tool_use_id").map(ToOwned::to_owned),
            model: get_str(event, "model").map(ToOwned::to_owned),
            config_path,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn request_text(&self) -> &str {
        match self.kind {
            SupervisorToolProxyKind::Command => self.command.trim(),
            SupervisorToolProxyKind::Ask => self.prompt.as_deref().unwrap_or("").trim(),
        }
    }

    fn need_text(&self) -> Option<&str> {
        self.need
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

fn shell_command_from_pre_tool_use(event: &Value) -> Option<&str> {
    if get_str(event, "hook_event_name") != Some("PreToolUse") {
        return None;
    }
    let tool_name = get_str(event, "tool_name")?;
    if !matches!(
        tool_name,
        "Bash" | "exec_command" | "functions.exec_command"
    ) {
        return None;
    }
    event
        .get("tool_input")
        .and_then(|input| get_str(input, "command").or_else(|| get_str(input, "cmd")))
}

fn payload_path(code_home: &Path, payload: &SupervisorToolProxyPayload) -> Result<PathBuf> {
    let turn = sanitize_id(payload.turn_id.as_deref().unwrap_or("turn"));
    let tool = sanitize_id(payload.tool_use_id.as_deref().unwrap_or("tool"));
    let dir = code_home.join(PAYLOAD_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir.join(format!("{turn}-{tool}.json")))
}

fn load_tool_proxy_config(path: &Path, root: &Path) -> Result<MixmodConfig> {
    if path.exists() && path.extension().and_then(|value| value.to_str()) == Some("json") {
        let value = read_json_file(path)?;
        return serde_json::from_value(value)
            .with_context(|| format!("failed to parse {}", path.display()));
    }
    load_config(root)
}

fn tool_proxy_task(payload: &SupervisorToolProxyPayload) -> Value {
    match payload.kind {
        SupervisorToolProxyKind::Command => {
            let command = payload.command.trim();
            let role = worker_role_for_command(command);
            let need = payload.need_text();
            let need_instruction = need
                .map(|need| {
                    format!(
                        "\n\nSupervisor information need:\n{need}\n\nThe local worker may summarize the captured output for this need, but Mixmod executes the shell command itself."
                    )
                })
                .unwrap_or_default();
            json!({
                "title": format!("Supervisor tool proxy: {command}"),
                "instructions": format!(
                    "A GPT supervisor requested this Bash command:\n\n```bash\n{command}\n```{need_instruction}\n\nMixmod executes the command deterministically from the current repository context and captures stdout, stderr, exit status, duration, and artifacts. The local worker is used only afterward to summarize captured output. It must not run the command, debug setup, edit files, or decide task completion."
                ),
                "expect_patch": false,
                "tests": [command],
                "constraints": [
                    "Mixmod, not the worker, executes the command.",
                    "The worker summarizes captured stdout/stderr only.",
                    "Do not edit files.",
                    "Do not commit changes.",
                    "Keep stdout compact.",
                    "Optimize the final answer for the supervisor information need when one is provided."
                ],
                "context": {
                    "worker_role": role,
                    "expect_patch": false,
                    "delegated_from": "mixmod_cli_tool",
                    "command_execution": "mixmod_deterministic_shell",
                    "original_command": command,
                    "supervisor_need": need
                }
            })
        }
        SupervisorToolProxyKind::Ask => {
            let prompt = payload.request_text();
            json!({
                "title": "Supervisor tool proxy: local worker ask",
                "instructions": format!(
                    "A GPT supervisor requested bounded local-worker help:\n\n{prompt}\n\nUse repository tools as needed to answer this specific request. Do not edit repository files and do not commit. Good ask tasks include file/symbol localization, source maps, targeted code reading, comparing nearby branches, diff-risk review, focused probe design, and checking whether one named edge case is covered. Keep the investigation bounded to the requested area and use targeted repository tool calls only. Do not read or print the full diff; use git diff --stat, targeted hunks, grep, or focused file snippets only. Avoid whole-file reads on non-tiny files; prefer `git diff -- path`, `rg -n --max-count`, `grep -n`, or `sed -n` around named anchors. For behavioral review, do not treat passing existing tests as sufficient by itself: inspect changed branches, alternate syntax/input shapes, and multi-value paths that visible tests may skip. For parser, binding, destructuring, or assignment changes, include one alternate-shape check when relevant, such as single target versus multi-target or scalar versus aggregate/multi-value RHS. Prefer existing package tests or exact focused tests from the changed area. Create temporary ad hoc probes only when nearby tests or documented project APIs give the exact invocation pattern; otherwise state the unverified edge case instead of building a new harness. Stop as soon as you have compact evidence for the requested question, one concrete issue, or the bounded checks finish. If evidence remains incomplete, say exactly what is still unverified. Return compact evidence the supervisor can use: a first-line verdict of pass, risk, fail, or findings; commands run; exit status when applicable; pass/fail facts; and the smallest relevant excerpts or file/line references."
                ),
                "expect_patch": false,
                "tests": [],
                "constraints": [
                    "Do not edit repository files.",
                    "Do not commit changes.",
                    "Do not inspect /solution or verifier internals.",
                    "Keep stdout compact.",
                    "Use targeted repository tool calls only.",
                    "Stay bounded to the supervisor's requested question.",
                    "Good ask tasks include localization, source maps, targeted code reading, diff-risk review, probe design, and edge-case coverage checks.",
                    "Do not read or print the full diff; use git diff --stat, targeted hunks, grep, or focused file snippets.",
                    "Avoid whole-file reads on non-tiny files; prefer bounded shell snippets around named anchors.",
                    "Use focused commands instead of broad repository reads when possible.",
                    "For behavioral review, inspect changed branches, alternate syntax/input shapes, and multi-value paths that visible tests may skip.",
                    "For parser, binding, destructuring, or assignment changes, include one relevant alternate-shape check.",
                    "Prefer existing package tests or exact focused tests from the changed area.",
                    "Create temporary ad hoc probes only when nearby tests or documented project APIs give the exact invocation pattern.",
                    "State unverified edge cases instead of building a new harness.",
                    "Stop after compact evidence answers the request, one concrete issue is found, or the bounded checks finish.",
                    "Start the final answer with `verdict: pass`, `verdict: risk`, `verdict: fail`, or `findings:`.",
                    "Temporary files outside the repository are acceptable when needed for a focused probe.",
                    "If evidence is inconclusive, say exactly what remains unverified."
                ],
                "context": {
                    "worker_role": "bounded_review",
                    "expect_patch": false,
                    "delegated_from": "mixmod_cli_tool",
                    "worker_prompt": prompt
                }
            })
        }
    }
}

fn worker_role_for_command(command: &str) -> &'static str {
    let command = command.trim();
    if command.starts_with("git diff") || command.starts_with("git status") {
        "diff_review"
    } else if is_test_or_build_command(command) {
        "run_checks"
    } else {
        "inspect"
    }
}

pub(crate) fn should_proxy_bash_command(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    if command_segments(command).any(is_tool_recursion_command) {
        return false;
    }
    if contains_obvious_mutation(command) {
        return false;
    }
    command_segments(command)
        .any(|segment| is_inspection_command(segment) || is_test_or_build_command(segment))
}

fn command_segments(command: &str) -> impl Iterator<Item = &str> {
    command
        .split(|ch| matches!(ch, '\n' | ';' | '|' | '&'))
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
}

fn is_tool_recursion_command(command: &str) -> bool {
    command == "mixmod"
        || command.starts_with("mixmod ")
        || command == "codex"
        || command.starts_with("codex ")
        || command == "opencode"
        || command.starts_with("opencode ")
        || command == "codex-hook"
        || command.starts_with("codex-hook ")
        || command.contains(" codex-hook ")
}

fn is_inspection_command(command: &str) -> bool {
    [
        "git diff",
        "git status",
        "git show",
        "git log",
        "git grep",
        "rg ",
        "grep ",
        "sed -n ",
        "cat ",
        "ls ",
        "find ",
    ]
    .iter()
    .any(|prefix| command == prefix.trim() || command.starts_with(prefix))
}

fn is_test_or_build_command(command: &str) -> bool {
    [
        "go test",
        "cargo test",
        "cargo check",
        "pytest",
        "python -m pytest",
        "npm test",
        "pnpm test",
        "yarn test",
        "make test",
    ]
    .iter()
    .any(|prefix| command == *prefix || command.starts_with(&format!("{prefix} ")))
}

fn is_git_status_command(command: &str) -> bool {
    command == "git status" || command.starts_with("git status ")
}

fn is_git_diff_check_command(command: &str) -> bool {
    command == "git diff --check" || command.starts_with("git diff --check ")
}

fn is_git_diff_stat_command(command: &str) -> bool {
    command == "git diff --stat" || command.starts_with("git diff --stat ")
}

fn contains_obvious_mutation(command: &str) -> bool {
    [
        "rm ",
        "mv ",
        "cp ",
        "touch ",
        "chmod ",
        "chown ",
        "git add",
        "git commit",
        "git checkout",
        "git reset",
        "git clean",
        "git apply",
        "gofmt ",
        "ruff --fix",
    ]
    .iter()
    .any(|token| command.starts_with(token) || command.contains(&format!(" {token}")))
}

fn extract_last_text(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| get_str(event, "type") == Some("text"))
        .filter_map(|event| {
            event
                .get("part")
                .and_then(|part| get_str(part, "text"))
                .map(ToOwned::to_owned)
        })
        .last()
}

fn compact_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.trim().to_string();
    }
    let mut start = text.len().saturating_sub(max_bytes);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    format!("<truncated>\n{}", text[start..].trim())
}

fn sanitize_id(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "item".to_string()
    } else {
        sanitized
    }
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxies_evidence_commands() {
        for command in [
            "git diff --stat",
            "git status --short",
            "git grep TypeBinding",
            "rg TypeBinding",
            "git status --short && rg -n TypeBinding vm parser",
            "python -m pytest tests/test_bindings.py -q 2>&1 | tail -80",
            "go test ./vm -run TestVar",
            "cargo test run_writes_full_artifact_bundle",
        ] {
            assert!(should_proxy_bash_command(command), "{command}");
        }
    }

    #[test]
    fn command_tool_task_requests_compact_search_summaries() {
        let temp = tempfile::tempdir().unwrap();
        let payload = SupervisorToolProxyPayload::from_command(
            "rg -n TypedBindings vm parser",
            Some("Return matching files and the most relevant line numbers only."),
            temp.path(),
        );

        let task = tool_proxy_task(&payload);
        let instructions = task["instructions"].as_str().unwrap();
        let constraints = task["constraints"].as_array().unwrap();

        assert!(instructions.contains("Supervisor information need"));
        assert!(instructions.contains("most relevant line numbers"));
        assert!(instructions.contains("Mixmod executes the command deterministically"));
        assert!(instructions.contains("used only afterward to summarize captured output"));
        assert!(constraints.iter().any(|value| {
            value
                .as_str()
                .unwrap_or("")
                .contains("Mixmod, not the worker")
        }));
        assert!(constraints.iter().any(|value| {
            value
                .as_str()
                .unwrap_or("")
                .contains("supervisor information need")
        }));
        assert_eq!(task["context"]["worker_role"], json!("inspect"));
        assert_eq!(
            task["context"]["command_execution"],
            json!("mixmod_deterministic_shell")
        );
        assert_eq!(
            task["context"]["supervisor_need"],
            json!("Return matching files and the most relevant line numbers only.")
        );
    }

    #[test]
    fn search_command_summary_writes_structured_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let logs = out_dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let stdout = "\
src/lib.rs:10:fn target_symbol() {}
src/lib.rs:20:target_symbol();
tests/lib.rs:5:assert!(target_symbol());
        ";
        std::fs::write(logs.join("command.stdout.txt"), stdout).unwrap();
        std::fs::write(logs.join("command.stderr.txt"), "").unwrap();
        std::fs::write(out_dir.join("command-result.json"), "{}").unwrap();
        let payload = SupervisorToolProxyPayload::from_command(
            "rg -n 'target_symbol' .",
            Some("Return files and first hits per file."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 12,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: 0,
            stdout_path: logs.join("command.stdout.txt"),
            stderr_path: logs.join("command.stderr.txt"),
        };

        let task = command_summary_task(&payload, temp.path(), &out_dir, &result).unwrap();

        let instructions = task["instructions"].as_str().unwrap();
        assert!(instructions.contains("Read only the listed command artifact files"));
        assert!(instructions.contains("Stdout artifact"));
        assert!(instructions.contains("Stderr artifact"));
        assert!(instructions.contains("Command result artifact"));
        assert!(instructions.contains("Search summary artifact"));
        assert!(instructions.contains("Do not copy raw stdout/stderr"));
        assert!(instructions.contains("do not return truncated output"));
        assert!(instructions.contains("search-summary.json"));
        assert!(!instructions.contains("Structured stdout summary"));
        assert!(!instructions.contains("Stdout excerpt"));
        assert!(!instructions.contains("Stderr excerpt"));
        assert!(!instructions.contains("src/lib.rs:10:fn target_symbol"));
        let readable_stdout = task["context"]["stdout_artifact"].as_str().unwrap();
        let readable_result = task["context"]["command_result_artifact"].as_str().unwrap();
        let readable_search = task["context"]["search_summary_artifact"].as_str().unwrap();
        assert!(readable_stdout.contains("xdg-data/opencode/tool-output"));
        assert!(readable_result.contains("xdg-data/opencode/tool-output"));
        assert!(readable_search.contains("xdg-data/opencode/tool-output"));
        assert!(Path::new(readable_stdout).exists());
        assert!(Path::new(readable_result).exists());
        assert!(Path::new(readable_search).exists());
        let summary = read_json_file(&out_dir.join("search-summary.json")).unwrap();
        assert_eq!(summary["kind"], json!("line_search"));
        assert_eq!(summary["total_output_lines"], json!(3));
        assert_eq!(summary["returned_files"], json!(2));
        assert_eq!(summary["files"][0]["path"], json!("src/lib.rs"));
        assert_eq!(summary["files"][0]["match_count"], json!(2));
        assert_eq!(summary["files"][1]["path"], json!("tests/lib.rs"));
    }

    #[test]
    fn compound_search_command_summary_writes_structured_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let logs = out_dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let stdout = "\
 M src/lib.rs
src/lib.rs:10:fn target_symbol() {}
src/other.rs:4:target_symbol();
";
        std::fs::write(logs.join("command.stdout.txt"), stdout).unwrap();
        std::fs::write(logs.join("command.stderr.txt"), "").unwrap();
        std::fs::write(out_dir.join("command-result.json"), "{}").unwrap();
        let payload = SupervisorToolProxyPayload::from_command(
            "git status --short && rg -n 'target_symbol' src",
            Some("Return changed files and relevant hits."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 20,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: 0,
            stdout_path: logs.join("command.stdout.txt"),
            stderr_path: logs.join("command.stderr.txt"),
        };

        let task = command_summary_task(&payload, temp.path(), &out_dir, &result).unwrap();

        let readable_search = task["context"]["search_summary_artifact"].as_str().unwrap();
        assert!(readable_search.contains("xdg-data/opencode/tool-output"));
        assert!(Path::new(readable_search).exists());
        let summary = read_json_file(&out_dir.join("search-summary.json")).unwrap();
        assert_eq!(summary["kind"], json!("line_search"));
        assert_eq!(summary["total_output_lines"], json!(3));
        assert_eq!(summary["returned_files"], json!(3));
        assert_eq!(summary["files"][0]["path"], json!("<stdout>"));
        assert_eq!(summary["files"][1]["path"], json!("src/lib.rs"));
    }

    #[test]
    fn find_command_summary_groups_paths() {
        let summary = search_output_summary("find . -name '*.rs'", "./src/lib.rs\n./tests/a.rs\n")
            .expect("find output should be summarized");

        assert_eq!(summary["kind"], json!("path_search"));
        assert_eq!(summary["total_output_lines"], json!(2));
        assert_eq!(summary["paths"][0], json!("./src/lib.rs"));
    }

    #[test]
    fn fallback_command_summary_points_to_artifacts_without_excerpts() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let logs = out_dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let stdout = "secret stdout line that should not be embedded\n";
        let stderr = "secret stderr line that should not be embedded\n";
        std::fs::write(logs.join("command.stdout.txt"), stdout).unwrap();
        std::fs::write(logs.join("command.stderr.txt"), stderr).unwrap();
        std::fs::write(out_dir.join("search-summary.json"), "{}").unwrap();
        let payload = SupervisorToolProxyPayload::from_command(
            "rg -n secret .",
            Some("Return whether secret appears."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 12,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: stderr.len() as u64,
            stdout_path: logs.join("command.stdout.txt"),
            stderr_path: logs.join("command.stderr.txt"),
        };

        let summary = fallback_command_summary(&payload, &out_dir, &result);

        assert!(summary.contains("summary_unavailable"));
        assert!(summary.contains("stdout_artifact: logs/command.stdout.txt"));
        assert!(summary.contains("stderr_artifact: logs/command.stderr.txt"));
        assert!(summary.contains("command_result: command-result.json"));
        assert!(summary.contains("search_summary: search-summary.json"));
        assert!(!summary.contains("secret stdout"));
        assert!(!summary.contains("secret stderr"));
    }

    #[test]
    fn deterministic_command_summary_handles_small_exact_output() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let logs = out_dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("command.stdout.txt"), "ok\n").unwrap();
        std::fs::write(logs.join("command.stderr.txt"), "").unwrap();
        let payload = SupervisorToolProxyPayload::from_command(
            "python -c 'print(\"ok\")'",
            Some("Return the exact short output."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 10,
            stdout_bytes: 3,
            stderr_bytes: 0,
            stdout_path: logs.join("command.stdout.txt"),
            stderr_path: logs.join("command.stderr.txt"),
        };

        let summary = deterministic_command_summary(&payload, &out_dir, &result)
            .unwrap()
            .expect("small output should be summarized directly");

        assert!(
            summary
                .text
                .contains("summary_source: mixmod_deterministic")
        );
        assert!(summary.text.contains("summary_kind: small_exact_output"));
        assert!(summary.text.contains("ok"));
        assert_eq!(
            summary.metrics.unwrap()["opencode_call"],
            json!(false),
            "direct summaries must not count as OpenCode calls"
        );
        assert!(out_dir.join(METRICS_JSON).exists());
        assert!(out_dir.join(REPORT_MD).exists());
    }

    #[test]
    fn deterministic_command_summary_handles_search_output() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let logs = out_dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let stdout = "\
src/lib.rs:10:fn target_symbol() {}
src/lib.rs:20:target_symbol();
tests/lib.rs:5:assert!(target_symbol());
";
        std::fs::write(logs.join("command.stdout.txt"), stdout).unwrap();
        std::fs::write(logs.join("command.stderr.txt"), "").unwrap();
        let payload = SupervisorToolProxyPayload::from_command(
            "rg -n target_symbol .",
            Some("Return grouped hits."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 10,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: 0,
            stdout_path: logs.join("command.stdout.txt"),
            stderr_path: logs.join("command.stderr.txt"),
        };

        let summary = deterministic_command_summary(&payload, &out_dir, &result)
            .unwrap()
            .expect("search output should be summarized directly");

        assert!(summary.text.contains("summary_kind: search_summary"));
        assert!(summary.text.contains("search_kind: line_search"));
        assert!(summary.text.contains("returned_files: 2"));
        assert!(summary.text.contains("search_summary: search-summary.json"));
        assert!(out_dir.join("search-summary.json").exists());
        assert_eq!(
            read_json_file(&out_dir.join(METRICS_JSON)).unwrap()["summary_source"],
            json!("mixmod_deterministic")
        );
    }

    #[test]
    fn deterministic_command_summary_leaves_large_ambiguous_output_to_worker() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let logs = out_dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let stdout = "x".repeat((DIRECT_SUMMARY_MAX_EXACT_STDOUT_BYTES + 1) as usize);
        std::fs::write(logs.join("command.stdout.txt"), stdout.as_bytes()).unwrap();
        std::fs::write(logs.join("command.stderr.txt"), "").unwrap();
        let payload = SupervisorToolProxyPayload::from_command(
            "python script.py",
            Some("Summarize the important semantic output."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 10,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: 0,
            stdout_path: logs.join("command.stdout.txt"),
            stderr_path: logs.join("command.stderr.txt"),
        };

        assert!(
            deterministic_command_summary(&payload, &out_dir, &result)
                .unwrap()
                .is_none()
        );
        assert!(!out_dir.join(METRICS_JSON).exists());
    }

    #[test]
    fn tool_proxy_summary_limits_keep_codex_output_compact() {
        assert!(COMMAND_SUMMARY_BYTES < ASK_SUMMARY_BYTES);
        assert!(COMMAND_SUMMARY_BYTES <= 2_000);
        assert!(ASK_SUMMARY_BYTES <= 3_000);
    }

    #[test]
    fn does_not_proxy_mutating_or_recursive_commands() {
        for command in [
            "gofmt -w vm/vm.go",
            "git add .",
            "git status --short && mixmod tool run-command --command 'rg x'",
            "git status --short && codex exec --help",
            "git status --short && opencode run hi",
            "mixmod codex-hook run-tool-proxy --payload x",
            "opencode run hi",
        ] {
            assert!(!should_proxy_bash_command(command), "{command}");
        }
    }

    #[test]
    fn pre_tool_use_command_parser_accepts_codex_exec_command_shape() {
        let event = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "exec_command",
            "tool_input": {
                "cmd": "git status --short"
            }
        });

        assert_eq!(
            shell_command_from_pre_tool_use(&event),
            Some("git status --short")
        );
    }

    #[test]
    fn tool_proxy_config_loader_uses_repo_config_for_toml_paths() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = state_layout(temp.path()).config();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(
            &config_path,
            toml::to_string(&MixmodConfig::default()).unwrap(),
        )
        .unwrap();

        let config = load_tool_proxy_config(&config_path, temp.path()).unwrap();

        assert_eq!(config.opencode.provider, "llama.cpp");
    }

    #[test]
    fn ask_tool_task_requests_bounded_behavior_review() {
        let temp = tempfile::tempdir().unwrap();
        let payload = SupervisorToolProxyPayload::from_prompt(
            "Review the final diff for missing behavior.",
            temp.path(),
        );

        let task = tool_proxy_task(&payload);
        let instructions = task["instructions"].as_str().unwrap();
        let constraints = task["constraints"].as_array().unwrap();

        assert!(instructions.contains("file/symbol localization"));
        assert!(instructions.contains("source maps"));
        assert!(instructions.contains("targeted code reading"));
        assert!(instructions.contains("focused probe design"));
        assert!(instructions.contains("checking whether one named edge case is covered"));
        assert!(instructions.contains("Do not read or print the full diff"));
        assert!(instructions.contains("Avoid whole-file reads"));
        assert!(instructions.contains("sed -n"));
        assert!(instructions.contains("inspect changed branches"));
        assert!(instructions.contains("visible tests may skip"));
        assert!(instructions.contains("alternate-shape check"));
        assert!(instructions.contains("multi-target"));
        assert!(instructions.contains("existing package tests"));
        assert!(instructions.contains("exact invocation pattern"));
        assert!(instructions.contains("unverified edge case"));
        assert!(instructions.contains("targeted repository tool calls"));
        assert!(instructions.contains("first-line verdict"));
        assert!(instructions.contains("findings"));
        assert!(
            constraints
                .iter()
                .any(|value| { value.as_str().unwrap_or("").contains("changed branches") })
        );
        assert!(constraints.iter().any(|value| {
            value
                .as_str()
                .unwrap_or("")
                .contains("alternate-shape check")
        }));
        assert!(
            constraints
                .iter()
                .any(|value| value.as_str().unwrap_or("").contains("whole-file reads"))
        );
        assert!(
            constraints
                .iter()
                .any(|value| value.as_str().unwrap_or("").contains("exact focused tests"))
        );
        assert!(constraints.iter().any(|value| {
            value
                .as_str()
                .unwrap_or("")
                .contains("exact invocation pattern")
        }));
        assert!(constraints.iter().any(|value| {
            value
                .as_str()
                .unwrap_or("")
                .contains("unverified edge cases")
        }));
        assert!(
            constraints
                .iter()
                .any(|value| { value.as_str().unwrap_or("").contains("targeted repository") })
        );
        assert!(
            constraints
                .iter()
                .any(|value| { value.as_str().unwrap_or("").contains("source maps") })
        );
        assert!(
            constraints
                .iter()
                .any(|value| value.as_str().unwrap_or("").contains("full diff"))
        );
        assert!(
            constraints
                .iter()
                .any(|value| value.as_str().unwrap_or("").contains("verdict: fail"))
        );
        assert_eq!(task["context"]["worker_role"], json!("bounded_review"));
    }

    #[test]
    fn ask_tool_caps_worker_timeouts_below_default() {
        let mut config = MixmodConfig::default();

        apply_tool_proxy_limits(SupervisorToolProxyKind::Ask, &mut config);

        assert_eq!(
            config.opencode.worker_timeout_seconds,
            ASK_WORKER_TIMEOUT_SECONDS
        );
        assert_eq!(
            config.opencode.idle_timeout_seconds,
            ASK_IDLE_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn command_tool_keeps_configured_timeouts() {
        let mut config = MixmodConfig::default();

        apply_tool_proxy_limits(SupervisorToolProxyKind::Command, &mut config);

        assert_eq!(config.opencode.worker_timeout_seconds, 600);
        assert_eq!(config.opencode.idle_timeout_seconds, 300);
    }

    #[test]
    fn command_summary_caps_worker_timeouts() {
        let mut config = MixmodConfig::default();

        apply_command_summary_limits(&mut config);

        assert_eq!(
            config.opencode.worker_timeout_seconds,
            COMMAND_SUMMARY_WORKER_TIMEOUT_SECONDS
        );
        assert_eq!(
            config.opencode.idle_timeout_seconds,
            COMMAND_SUMMARY_IDLE_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn command_tool_status_uses_inner_command_exit() {
        let temp = tempfile::tempdir().unwrap();
        let events = r#"
{"type":"tool_use","part":{"tool":"bash","state":{"input":{"command":"go test ./..."},"metadata":{"exit":1}}}}
{"type":"tool_use","part":{"tool":"bash","state":{"input":{"command":"git status --short"},"metadata":{"exit":0}}}}
"#;
        std::fs::write(temp.path().join(TOOL_EVENTS_JSONL), events).unwrap();

        let failed = command_tool_result(temp.path(), "go test ./...").unwrap();
        assert_eq!(failed.exit_status, Some(1));
        assert_eq!(tool_proxy_command_status(&failed), "command_failed");

        let passed = command_tool_result(temp.path(), "git status --short").unwrap();
        assert_eq!(passed.exit_status, Some(0));
        assert_eq!(tool_proxy_command_status(&passed), "success");
    }

    #[test]
    fn command_tool_runs_shell_directly_and_writes_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let config = MixmodConfig::default();

        let result =
            run_command_directly(temp.path(), &out_dir, "printf 'hello\\n'", &config).unwrap();

        assert_eq!(result.exit_status, Some(0));
        assert_eq!(result.stdout_bytes, 6);
        assert_eq!(
            std::fs::read_to_string(out_dir.join("logs/command.stdout.txt")).unwrap(),
            "hello\n"
        );
        assert!(out_dir.join("logs/command.stderr.txt").exists());
        assert!(out_dir.join("command-result.json").exists());
        assert!(out_dir.join(TOOL_EVENTS_JSONL).exists());
    }

    #[test]
    fn command_tool_result_stdout_is_compact() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let payload = SupervisorToolProxyPayload::from_command(
            "git diff --check",
            Some("Report only pass/fail and whitespace errors."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 5,
            stdout_bytes: 21,
            stderr_bytes: 0,
            stdout_path: out_dir.join("logs/command.stdout.txt"),
            stderr_path: out_dir.join("logs/command.stderr.txt"),
        };
        let empty_stats = crate::PatchStats::default();

        let output = render_command_tool_result(
            &payload,
            temp.path(),
            &out_dir,
            &json!({"opencode_exit_status": 0}),
            &result,
            &empty_stats,
            Some(&empty_stats),
            "success",
            "No whitespace errors.",
        );

        assert!(output.contains("Mixmod command proxy result"));
        assert!(output.contains("status: success"));
        assert!(output.contains("command_exit_status: 0"));
        assert!(output.contains("command_timed_out: false"));
        assert!(output.contains("command_stdout: logs/command.stdout.txt"));
        assert!(output.contains("need: Report only pass/fail"));
        assert!(output.contains("artifacts_dir:"));
        assert!(output.contains("artifacts: inspect artifacts_dir/{logs/command.stdout.txt"));
        assert!(output.contains("command-result.json"));
        assert!(output.contains("tool-events.jsonl"));
        assert!(output.contains("worktree.patch} if answer is insufficient"));
        assert!(output.contains("answer:\nNo whitespace errors."));
        assert!(!output.contains("prompt_artifact:"));
        assert!(!output.contains("report_artifact:"));
        assert!(!output.contains("tool_events_artifact:"));
        assert!(!output.contains("worker_summary:"));
    }

    #[test]
    fn command_tool_result_replaces_overlong_worker_answer() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        let payload = SupervisorToolProxyPayload::from_command(
            "python -m pytest tests -q",
            Some("Report pass/fail."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 5,
            stdout_bytes: 21,
            stderr_bytes: 0,
            stdout_path: out_dir.join("logs/command.stdout.txt"),
            stderr_path: out_dir.join("logs/command.stderr.txt"),
        };
        let empty_stats = crate::PatchStats::default();
        let long_answer = format!(
            "{}TAIL_SHOULD_NOT_APPEAR",
            "summary line\n".repeat(COMMAND_SUMMARY_BYTES)
        );

        let output = render_command_tool_result(
            &payload,
            temp.path(),
            &out_dir,
            &json!({"opencode_exit_status": 0}),
            &result,
            &empty_stats,
            Some(&empty_stats),
            "success",
            &long_answer,
        );

        assert!(output.contains("summary_too_long"));
        assert!(output.contains("logs/opencode.stdout.txt"));
        assert!(!output.contains("TAIL_SHOULD_NOT_APPEAR"));
        assert!(!output.contains("<truncated>"));
    }

    #[test]
    fn command_tool_result_names_search_summary_when_present() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("tool-run");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(out_dir.join("search-summary.json"), "{}").unwrap();
        let payload = SupervisorToolProxyPayload::from_command(
            "git status --short && rg -n target src",
            Some("Return changed files and first hits."),
            temp.path(),
        );
        let result = CommandToolResult {
            exit_status: Some(0),
            timed_out: false,
            duration_ms: 5,
            stdout_bytes: 21,
            stderr_bytes: 0,
            stdout_path: out_dir.join("logs/command.stdout.txt"),
            stderr_path: out_dir.join("logs/command.stderr.txt"),
        };
        let empty_stats = crate::PatchStats::default();

        let output = render_command_tool_result(
            &payload,
            temp.path(),
            &out_dir,
            &json!({"opencode_exit_status": 0}),
            &result,
            &empty_stats,
            Some(&empty_stats),
            "success",
            "See structured search summary.",
        );

        assert!(output.contains("search_summary: search-summary.json"));
    }

    #[test]
    fn ask_tool_result_stdout_is_compact() {
        let temp = tempfile::tempdir().unwrap();
        let out_dir = temp.path().join("ask-run");
        let payload = SupervisorToolProxyPayload::from_prompt(
            "Review the final diff for missing behavior.",
            temp.path(),
        );
        let receipt = receipt_with_changed_files(Vec::new());

        let output = render_ask_tool_result(
            &payload,
            temp.path(),
            &out_dir,
            &json!({"opencode_exit_status": 0}),
            &receipt,
            "success",
            "verdict: pass\nNo missing edge case found in the bounded review.",
        );

        assert!(output.contains("Mixmod ask proxy result"));
        assert!(output.contains("request: Review the final diff"));
        assert!(output.contains("status: success"));
        assert!(output.contains("worker_exit_status: 0"));
        assert!(output.contains("artifacts_dir:"));
        assert!(output.contains("artifacts: inspect artifacts_dir/{logs/opencode.stdout.txt"));
        assert!(output.contains("reasoning-trace.jsonl"));
        assert!(output.contains("worktree.patch} if answer is insufficient"));
        assert!(output.contains("answer:\nverdict: pass"));
        assert!(!output.contains("prompt_artifact:"));
        assert!(!output.contains("report_artifact:"));
        assert!(!output.contains("tool_events_artifact:"));
        assert!(!output.contains("worker_summary:"));
    }

    #[test]
    fn side_effect_notice_reports_unexpected_worker_changes() {
        let mut receipt = receipt_with_changed_files(vec![
            "env/envValues.go".to_string(),
            "vm/vmTypedBindings_test.go".to_string(),
        ]);

        let notice = tool_proxy_side_effect_notice(
            &receipt,
            &json!({"changed_file_count": 2, "changed_line_count": 1716}),
        )
        .unwrap();

        assert!(notice.contains("changed_files=2"));
        assert!(notice.contains("changed_lines=1716"));
        assert!(notice.contains("env/envValues.go"));
        assert!(notice.contains("vm/vmTypedBindings_test.go"));

        receipt.changed_files.clear();
        assert!(tool_proxy_side_effect_notice(&receipt, &json!({})).is_none());
    }

    fn receipt_with_changed_files(changed_files: Vec<String>) -> crate::Receipt {
        crate::Receipt {
            run_id: "run-test".to_string(),
            status: "needs_supervisor".to_string(),
            mode: "explore".to_string(),
            summary: String::new(),
            changed_files,
            report: String::new(),
            patch: String::new(),
            worktree_patch: String::new(),
            session: String::new(),
            interventions: String::new(),
            metrics: String::new(),
            logs: String::new(),
        }
    }
}
