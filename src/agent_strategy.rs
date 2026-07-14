use std::path::Path;
use std::time::{Instant, SystemTime};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::json;

use crate::task::{read_task_json, write_agent_visible_task_file};
use crate::{
    FINAL_PATCH, METRICS_JSON, MixmodConfig, ModelOverrides, REPORT_MD, SupervisorCodexSession,
    TASK_JSON, TOOL_OUTPUT_DIR, WORKTREE_PATCH, WorkerSupervisorGuidance, absolutize, atomic_write,
    budgeted_report, display_path, get_str, git_diff_with_untracked, load_config, patch_stats,
    read_json_file, state_layout, write_pretty_json,
};

/// Options for the primary Codex-agent strategy.
pub(crate) struct AgentStrategyOptions {
    /// Per-run model choices supplied by CLI flags.
    pub(crate) model_overrides: ModelOverrides,
    /// Disable local-inference verification for worker-tool calls.
    pub(crate) no_require_local: bool,
}

/// Run the primary Mixmod strategy: Codex owns the solve and Mixmod provides
/// low-cost local worker routing as a tool.
pub(crate) fn run_agent_strategy(
    root: &Path,
    task_arg: &Path,
    out_dir: &Path,
    options: AgentStrategyOptions,
) -> Result<()> {
    AgentStrategyRun {
        root,
        task_arg,
        out_dir,
        options,
    }
    .execute()
}

struct AgentStrategyRun<'a> {
    root: &'a Path,
    task_arg: &'a Path,
    out_dir: &'a Path,
    options: AgentStrategyOptions,
}

impl AgentStrategyRun<'_> {
    fn execute(self) -> Result<()> {
        let Self {
            root,
            task_arg,
            out_dir,
            options,
        } = self;
        let run_start = Utc::now();
        let run_start_time = SystemTime::now();
        let start = Instant::now();
        let out_dir = absolutize(root, out_dir);
        fs::create_dir_all(&out_dir).with_context(|| {
            format!("failed to create agent strategy dir {}", out_dir.display())
        })?;

        let mut config = load_config(root)?;
        options.model_overrides.apply_to_config(&mut config)?;
        if options.no_require_local {
            config.opencode.require_local = false;
            config.opencode.local_verification.enabled = false;
        }

        let task_file = out_dir.join(TASK_JSON);
        write_agent_visible_task_file(&absolutize(root, task_arg), &task_file)?;
        let (task_json, _task) = read_task_json(&task_file)?;

        let worker_tool_guide_path = out_dir.join("worker-tool-guide.md");
        let worker_tool_guide = render_worker_tool_guide(root, &worker_tool_guide_path, &config);
        atomic_write(&worker_tool_guide_path, worker_tool_guide.as_bytes())?;
        let prompt = agent_strategy_prompt(
            root,
            &task_file,
            &worker_tool_guide_path,
            &task_json,
            &config,
        );
        atomic_write(&out_dir.join("agent-prompt.md"), prompt.as_bytes())?;

        let supervisor = config.supervisor.clone();
        let mut supervisor_session =
            SupervisorCodexSession::start(root, &supervisor, Some(&config))?;
        let result = supervisor_session.run_turn(&out_dir, "agent", &prompt)?;

        let patch = git_diff_with_untracked(root).unwrap_or_else(|error| {
            format!("Unable to capture git diff after Codex agent run: {error}\n")
        });
        atomic_write(&out_dir.join(FINAL_PATCH), patch.as_bytes())?;
        atomic_write(&out_dir.join(WORKTREE_PATCH), patch.as_bytes())?;
        let stats = patch_stats(&patch);
        let changed_files = stats.files.clone();
        let success = result.exit_status == Some(0);
        let codex_turn_status = result.turn_status.clone();
        let codex_error_info = result.error_info.clone();
        let codex_error_message = result.error_message.clone();
        let final_status = agent_final_status(success, codex_error_info.as_deref());
        let supervisor_token_usage_scope = if result.token_usage_comparable {
            "cumulative"
        } else {
            result.token_usage_scope.as_str()
        };
        let worker_tool_metrics = worker_tool_proxy_metrics(root, run_start_time);
        let codex_command_metrics = codex_command_metrics(&out_dir.join("logs/codex-agent.jsonl"));
        let agent_prompt_bytes = file_len_or_zero(&out_dir.join("agent-prompt.md"));
        let worker_tool_guide_bytes = file_len_or_zero(&worker_tool_guide_path);
        let metrics = json!({
            "kind": "mixmod-agent-strategy",
            "recorded_at": Utc::now().to_rfc3339(),
            "start_timestamp": run_start.to_rfc3339(),
            "end_timestamp": Utc::now().to_rfc3339(),
            "wall_clock_ms": start.elapsed().as_millis(),
            "supervisor_model": supervisor.model,
            "supervisor_reasoning_effort": supervisor.reasoning_effort,
            "supervisor_input_tokens": result.usage.input_tokens,
            "supervisor_output_tokens": result.usage.output_tokens,
            "supervisor_reasoning_tokens": result.usage.reasoning_tokens,
            "supervisor_total_tokens": result.usage.total_tokens,
            "supervisor_cached_input_tokens": result.usage.cached_input_tokens,
            "supervisor_input_bytes_fallback": result.input_bytes,
            "supervisor_output_bytes_fallback": result.output_bytes,
            "codex_visible_bytes": result.input_bytes,
            "agent_prompt_bytes": agent_prompt_bytes,
            "worker_tool_guide_bytes": worker_tool_guide_bytes,
            "supervision_turn_count": 1,
            "codex_calls": 1,
            "codex_backend": "app-server-persistent",
            "codex_app_server_thread_ids": [result.thread_id],
            "codex_app_server_turn_ids": [result.turn_id],
            "codex_app_server_thread_count": 1,
            "codex_turn_status": codex_turn_status,
            "codex_error_info": codex_error_info,
            "codex_error_message": codex_error_message,
            "supervisor_token_usage_source": result.token_usage_source,
            "supervisor_token_usage_scope": supervisor_token_usage_scope,
            "supervisor_token_usage_comparable": result.token_usage_comparable,
            "supervisor_session_reused": false,
            "supervisor_resume_count": 0,
            "strategy_phases": ["codex_primary_agent"],
            "mixmod_delegations": worker_tool_metrics.call_count,
            "opencode_calls": worker_tool_metrics.opencode_call_count,
            "deterministic_command_summaries": worker_tool_metrics.deterministic_command_count,
            "artifact_only_command_summaries": worker_tool_metrics.artifact_only_command_count,
            "supervisor_tool_proxy_enabled": config.strategy.supervisor_tool_proxy.enabled,
            "codex_command_count": codex_command_metrics.command_count,
            "codex_direct_command_count": codex_command_metrics.direct_command_count,
            "codex_routed_command_count": codex_command_metrics.routed_command_count,
            "codex_command_output_bytes": codex_command_metrics.output_bytes,
            "codex_direct_command_output_bytes": codex_command_metrics.direct_output_bytes,
            "codex_routed_command_output_bytes": codex_command_metrics.routed_output_bytes,
            "worker_backend": config.worker.backend.as_str(),
            "opencode_provider": config.opencode.provider,
            "opencode_model": config.opencode.model,
            "opencode_model_arg": format!("{}/{}", config.opencode.provider, config.opencode.model),
            "require_local": config.opencode.require_local,
            "local_inference_verified": worker_tool_metrics.opencode_call_count > 0 && config.opencode.require_local,
            "local_worker_stdout_bytes": worker_tool_metrics.stdout_bytes,
            "local_worker_stderr_bytes": worker_tool_metrics.stderr_bytes,
            "local_worker_text_bytes": worker_tool_metrics.stdout_bytes + worker_tool_metrics.stderr_bytes,
            "local_worker_reasoning_trace_bytes": worker_tool_metrics.reasoning_trace_bytes,
            "local_worker_tool_events_bytes": worker_tool_metrics.tool_events_bytes,
            "local_worker_tool_output_artifact_count": worker_tool_metrics.tool_output_artifact_count,
            "local_worker_tool_output_artifact_bytes": worker_tool_metrics.tool_output_artifact_bytes,
            "artifact_byte_sizes": artifact_byte_sizes(&out_dir)?,
            "patch_bytes": patch.len() as u64,
            "changed_files": changed_files,
            "changed_file_count": stats.files.len(),
            "changed_line_count": stats.changed_line_count,
            "final_status": final_status,
            "final_verdict": if success { "completed" } else { "failed" },
            "final_codex_action": if success { "completed" } else { "failed" },
            "terminal_reject": false,
            "needs_worker_revision": false,
            "notes": [
                "Agent strategy runs Codex as the primary task-solving agent.",
                "Mixmod provides a CLI tool that can route bounded command evidence requests to the configured worker.",
                "The worker is a low-cost tool; it is not the owner of the implementation loop."
            ]
        });
        write_pretty_json(
            &out_dir.join(METRICS_JSON),
            &metrics,
            "agent strategy metrics",
        )?;
        atomic_write(
            &out_dir.join(REPORT_MD),
            budgeted_report("exec", &metrics).as_bytes(),
        )?;

        println!(
            "Mixmod exec wrote artifacts to {}",
            display_path(root, &out_dir)
        );
        println!("status: {}", final_status);
        println!("report: {}", display_path(root, &out_dir.join(REPORT_MD)));
        println!("patch: {}", display_path(root, &out_dir.join(FINAL_PATCH)));
        if !success {
            bail!(
                "Codex primary-agent turn failed; artifacts were written to {}",
                out_dir.display()
            );
        }
        Ok(())
    }
}

fn agent_final_status(success: bool, codex_error_info: Option<&str>) -> &'static str {
    if success {
        "completed_by_codex"
    } else if codex_error_info == Some("usageLimitExceeded") {
        "codex_usage_limit_exceeded"
    } else if codex_error_info.is_some() {
        "codex_system_error"
    } else {
        "codex_failed"
    }
}

fn agent_strategy_prompt(
    root: &Path,
    task_file: &Path,
    worker_tool_guide_path: &Path,
    task_json: &serde_json::Value,
    config: &MixmodConfig,
) -> String {
    let mixmod_tool_command = env::current_exe()
        .map(|path| shell_quote_path(&path))
        .unwrap_or_else(|_| "mixmod".to_string());
    let worker_summary = worker_tool_prompt_summary(&config.worker_supervisor_guidance());
    let title = task_json
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("Mixmod task");
    let instructions = task_json
        .get("instructions")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    format!(
        r#"You are the primary Codex agent for this Mixmod run.

Complete the task in the working repository. You may inspect files, edit files,
run focused checks, and use git commands. Do not ask the user for approval. Do
not commit unless the task explicitly requires a commit. Leave the final
solution as a git diff so Mixmod can capture it.

Mixmod is a tool/router for a cheap local worker. It is not a separate
supervisor. The local worker is useful for bounded evidence gathering and weak
for final judgment. Treat its answers as fallible evidence; final correctness is
your responsibility.

Economic rule: do not run Bash commands directly. Execute shell work through
Mixmod with `tool run-command`, and always include `--need` with the exact
compact evidence you want back. Mixmod runs the exact command, captures
stdout/stderr/result artifacts, and returns a compact answer. It may ask the
cheap local worker to summarize large or semantic command outputs when that is
likely to reduce GPT context. Search output is especially good to route this way.
For broad searches, add `--select --page-size 8` so the local worker ranks the
most relevant hits first; request later pages only if page 1 is insufficient.

Primary helper command:

```bash
{mixmod_tool_command} tool run-command --command "git status --short" --need "Return tracked change status only."
```

Broad search helper:

```bash
{mixmod_tool_command} tool run-command --command "rg -n target src tests" --need "Rank likely relevant implementation and test hits." --select --page-size 8
```

`--need` is required. Use it to request the exact compact fact you need:
pass/fail, failing test name, grouped search hits, changed files, or notable
hunks. After routing a command, wait for the returned result rather than polling.

Use `tool ask` sparingly for one bounded non-command question, such as likely
files/symbols, one source-path summary, one diff risk, or one probe idea:

```bash
{mixmod_tool_command} tool ask --prompt "Inspect one named file or hunk for one named risk. Return compact evidence only."
```

Local worker profile: {worker_summary}

Detailed helper guidance is available at: {worker_tool_guide_path}
Read that file only if the compact rules above are insufficient.

Before final approval, verify the requested behavior using primary evidence:
changed source, diff hunks, command exit statuses, focused probes, or package
checks. Do not treat local-worker approval as completion.

Before finishing, check the resulting diff against the original task. If checks
cannot run, record the blocker in your final response. Your final response should
be concise and include what changed and what verification you performed.

Working repo: {root}
Task artifact: {task_file}

Task title: {title}

Task instructions:
{instructions}
"#,
        root = root.display(),
        task_file = task_file.display(),
        worker_tool_guide_path = worker_tool_guide_path.display(),
    )
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn render_worker_tool_guidance(guidance: &WorkerSupervisorGuidance) -> String {
    if guidance.guidance.is_empty() {
        return "- No worker-specific profile is configured.".to_string();
    }
    let mut lines = vec![format!("- Worker model/profile: {}", guidance.model)];
    lines.extend(guidance.guidance.iter().map(|item| format!("- {item}")));
    lines.join("\n")
}

fn render_worker_tool_guide(root: &Path, guide_path: &Path, config: &MixmodConfig) -> String {
    let mixmod_tool_command = env::current_exe()
        .map(|path| shell_quote_path(&path))
        .unwrap_or_else(|_| "mixmod".to_string());
    let worker_guidance = render_worker_tool_guidance(&config.worker_supervisor_guidance());
    format!(
        r#"# Mixmod Local Helper Guide

This artifact is optional reference material for the Codex primary agent.

Working repo: {root}
Guide artifact: {guide_path}

## Core Contract

Mixmod routes bounded work to the configured local worker. Codex remains the
primary implementation agent and final correctness authority.

Do not run Bash commands directly. Execute shell work through Mixmod with
`tool run-command` and an explicit `--need` information request:

```bash
{mixmod_tool_command} tool run-command --command "rg -n target src tests" --need "Return grouped matching files and first relevant line numbers."
```

`--need` is required. `tool run-command` executes the exact shell command
through Mixmod, captures stdout/stderr/result files, and returns a compact
answer plus an artifact directory. The full stdout/stderr stay on disk. Mixmod
may ask the cheap local worker to summarize large or semantic output,
especially searches and failing checks. If the compact answer is insufficient,
inspect the named artifact file.

For broad searches, request ranked selection:

```bash
{mixmod_tool_command} tool run-command --command "rg -n target src tests" --need "Rank likely relevant implementation and test hits." --select --page-size 8
```

The first call returns page 1 and a `selection_artifact`. If more candidates are
needed, use the returned `tool selection-page --selection ... --page N` command
instead of rerunning the search or asking the worker to rank again.

Prefer `tool ask` only for one bounded non-command request where a weak local
model can save context: localize likely files or symbols, summarize one named
source path, inspect one hunk for one risk, or propose one focused probe.

Avoid broad `tool ask` reviews. Use concrete routed commands for final evidence
when possible.

## Worker Profile

{worker_guidance}
"#,
        root = root.display(),
        guide_path = guide_path.display(),
    )
}

fn worker_tool_prompt_summary(guidance: &WorkerSupervisorGuidance) -> String {
    if guidance.model.trim().is_empty() {
        return "no model-specific profile configured; use only bounded helper calls".to_string();
    }
    format!(
        "{}; cheap local helper; use for bounded evidence, not final judgment",
        guidance.model
    )
}

#[derive(Default)]
struct WorkerToolProxyMetrics {
    call_count: u64,
    opencode_call_count: u64,
    deterministic_command_count: u64,
    artifact_only_command_count: u64,
    stdout_bytes: u64,
    stderr_bytes: u64,
    reasoning_trace_bytes: u64,
    tool_events_bytes: u64,
    tool_output_artifact_count: u64,
    tool_output_artifact_bytes: u64,
}

#[derive(Default)]
struct CodexCommandMetrics {
    command_count: u64,
    direct_command_count: u64,
    routed_command_count: u64,
    output_bytes: u64,
    direct_output_bytes: u64,
    routed_output_bytes: u64,
}

fn codex_command_metrics(path: &Path) -> CodexCommandMetrics {
    let mut metrics = CodexCommandMetrics::default();
    let Ok(text) = fs::read_to_string(path) else {
        return metrics;
    };
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(item) = event.get("params").and_then(|params| params.get("item")) else {
            continue;
        };
        if get_str(&event, "method") != Some("item/completed")
            || get_str(item, "type") != Some("commandExecution")
        {
            continue;
        }
        let command = get_str(item, "command").unwrap_or("");
        let output_bytes = item
            .get("aggregatedOutput")
            .and_then(|value| value.as_str())
            .map(|value| value.len() as u64)
            .unwrap_or(0);
        metrics.command_count += 1;
        metrics.output_bytes += output_bytes;
        if is_mixmod_routed_command(command) {
            metrics.routed_command_count += 1;
            metrics.routed_output_bytes += output_bytes;
        } else {
            metrics.direct_command_count += 1;
            metrics.direct_output_bytes += output_bytes;
        }
    }
    metrics
}

fn is_mixmod_routed_command(command: &str) -> bool {
    command.contains(" tool run-command ")
        || command.contains(" tool ask ")
        || command.contains(" codex-hook run-tool-proxy ")
}

fn worker_tool_proxy_metrics(root: &Path, since: SystemTime) -> WorkerToolProxyMetrics {
    let mut metrics = WorkerToolProxyMetrics::default();
    let proxy_root = state_layout(root)
        .project_dir()
        .join("supervisor-tool-proxy");
    let Ok(entries) = fs::read_dir(proxy_root) else {
        return metrics;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        for run in walk_dirs(&path) {
            let metrics_path = run.join(METRICS_JSON);
            if metrics_path.exists() && modified_at_or_after(&metrics_path, since) {
                let run_metrics = read_json_file(&metrics_path).ok();
                metrics.call_count += 1;
                if tool_proxy_metrics_is_opencode_call(run_metrics.as_ref()) {
                    metrics.opencode_call_count += 1;
                    metrics.stdout_bytes += file_len_or_zero(&run.join("logs/opencode.stdout.txt"));
                    metrics.stderr_bytes += file_len_or_zero(&run.join("logs/opencode.stderr.txt"));
                    metrics.reasoning_trace_bytes +=
                        file_len_or_zero(&run.join("reasoning-trace.jsonl"));
                } else if tool_proxy_metrics_is_deterministic_command(run_metrics.as_ref()) {
                    metrics.deterministic_command_count += 1;
                } else if tool_proxy_metrics_is_artifact_only_command(run_metrics.as_ref()) {
                    metrics.artifact_only_command_count += 1;
                }
                metrics.tool_events_bytes += file_len_or_zero(&run.join("tool-events.jsonl"));
                let tool_output = tool_output_artifact_metrics(&run.join(TOOL_OUTPUT_DIR));
                metrics.tool_output_artifact_count += tool_output.count;
                metrics.tool_output_artifact_bytes += tool_output.bytes;
            }
        }
    }
    metrics
}

fn tool_proxy_metrics_is_opencode_call(metrics: Option<&serde_json::Value>) -> bool {
    let Some(metrics) = metrics else {
        return true;
    };
    if let Some(opencode_call) = metrics
        .get("opencode_call")
        .and_then(|value| value.as_bool())
    {
        return opencode_call;
    }
    metrics.get("opencode_exit_status").is_some()
        || metrics
            .get("worker_backend")
            .and_then(|value| value.as_str())
            == Some("opencode")
}

fn tool_proxy_metrics_is_deterministic_command(metrics: Option<&serde_json::Value>) -> bool {
    metrics
        .and_then(|metrics| metrics.get("summary_source"))
        .and_then(|value| value.as_str())
        == Some("mixmod_deterministic")
}

fn tool_proxy_metrics_is_artifact_only_command(metrics: Option<&serde_json::Value>) -> bool {
    metrics
        .and_then(|metrics| metrics.get("summary_source"))
        .and_then(|value| value.as_str())
        == Some("mixmod_artifact_only")
}

#[derive(Default)]
struct FileTreeMetrics {
    count: u64,
    bytes: u64,
}

fn tool_output_artifact_metrics(path: &Path) -> FileTreeMetrics {
    let mut metrics = FileTreeMetrics::default();
    if !path.exists() {
        return metrics;
    }
    for file in walk_files(path) {
        metrics.count += 1;
        metrics.bytes += file_len_or_zero(&file);
    }
    metrics
}

fn walk_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = stack.pop() {
        if path.is_file() {
            files.push(path);
            continue;
        }
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            stack.push(entry.path());
        }
    }
    files
}

fn walk_dirs(root: &Path) -> Vec<std::path::PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut dirs = Vec::new();
    while let Some(dir) = stack.pop() {
        dirs.push(dir.clone());
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    dirs
}

fn file_len_or_zero(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn modified_at_or_after(path: &Path, since: SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| modified >= since)
        .unwrap_or(false)
}

fn artifact_byte_sizes(dir: &Path) -> Result<serde_json::Value> {
    let mut sizes = serde_json::Map::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
        {
            sizes.insert(name.to_string(), json!(entry.metadata()?.len()));
        }
    }
    Ok(serde_json::Value::Object(sizes))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn agent_strategy_prompt_makes_codex_primary_and_worker_a_tool() {
        let config = MixmodConfig::default();
        let prompt = agent_strategy_prompt(
            Path::new("/repo"),
            Path::new("/state/task.json"),
            Path::new("/state/worker-tool-guide.md"),
            &json!({
                "title": "Test task",
                "instructions": "Change the code."
            }),
            &config,
        );

        assert!(prompt.contains("You are the primary Codex agent"));
        assert!(prompt.contains("Mixmod is a tool/router"));
        assert!(prompt.contains("Economic rule"));
        assert!(prompt.contains("tool run-command"));
        assert!(prompt.contains("--need"));
        assert!(prompt.contains("--select --page-size 8"));
        assert!(prompt.contains("tool ask"));
        assert!(prompt.contains("/state/worker-tool-guide.md"));
        assert!(prompt.contains("Local worker profile:"));
        assert!(prompt.contains("final correctness"));
        assert!(prompt.len() < 5_000);
        assert!(!prompt.contains("Worker guidance:"));
        assert!(!prompt.contains("small_patch_slice"));
        assert!(!prompt.contains("alias/key generated-code repairs"));
        assert!(prompt.contains("tool run-command"));
        assert!(prompt.contains("tool ask"));
        assert!(!prompt.contains("Return only JSON"));
        assert!(!prompt.contains("\"action\":\"approve|revise|stop\""));

        let guide = render_worker_tool_guide(
            Path::new("/repo"),
            Path::new("/state/worker-tool-guide.md"),
            &config,
        );
        assert!(guide.contains("Mixmod Local Helper Guide"));
        assert!(guide.contains("Worker Profile"));
        assert!(guide.contains("selection-page"));
        assert!(guide.contains("small_patch_slice"));
        assert!(guide.contains("qwen-3.6-27b"));
    }

    #[test]
    fn agent_final_status_distinguishes_codex_usage_limits() {
        assert_eq!(agent_final_status(true, None), "completed_by_codex");
        assert_eq!(
            agent_final_status(false, Some("usageLimitExceeded")),
            "codex_usage_limit_exceeded"
        );
        assert_eq!(
            agent_final_status(false, Some("serverError")),
            "codex_system_error"
        );
        assert_eq!(agent_final_status(false, None), "codex_failed");
    }

    #[test]
    fn worker_tool_proxy_metrics_split_deterministic_and_opencode_calls() {
        let temp = tempfile::tempdir().unwrap();
        let since = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap();
        let proxy_root = state_layout(temp.path())
            .project_dir()
            .join("supervisor-tool-proxy")
            .join("cli");
        let deterministic = proxy_root.join("tool-fast");
        let opencode = proxy_root.join("tool-worker");
        std::fs::create_dir_all(&deterministic).unwrap();
        std::fs::create_dir_all(opencode.join("logs")).unwrap();
        write_pretty_json(
            &deterministic.join(METRICS_JSON),
            &json!({
                "summary_source": "mixmod_deterministic",
                "opencode_call": false
            }),
            "deterministic metrics",
        )
        .unwrap();
        std::fs::write(deterministic.join("tool-events.jsonl"), "{}\n").unwrap();
        write_pretty_json(
            &opencode.join(METRICS_JSON),
            &json!({
                "worker_backend": "opencode",
                "opencode_exit_status": 0
            }),
            "opencode metrics",
        )
        .unwrap();
        std::fs::write(opencode.join("logs/opencode.stdout.txt"), "worker").unwrap();

        let metrics = worker_tool_proxy_metrics(temp.path(), since);

        assert_eq!(metrics.call_count, 2);
        assert_eq!(metrics.opencode_call_count, 1);
        assert_eq!(metrics.deterministic_command_count, 1);
        assert_eq!(metrics.stdout_bytes, 6);
        assert_eq!(metrics.tool_events_bytes, 3);
    }
}
