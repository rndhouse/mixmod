use crate::*;

pub(crate) fn codex_only_prompt(work_dir: &Path, task: &Value) -> Result<String> {
    let visible_task = agent_visible_task_value(task);
    let task_json = serde_json::to_string_pretty(&visible_task)
        .context("failed to serialize agent-visible task for Codex-only prompt")?;
    Ok(format!(
        r#"You are the Codex-only baseline for a Mixmod experiment.
Solve the task directly in this repo. Edit files as needed, run the requested tests, and keep the final answer compact.
Do not use Mixmod or OpenCode.
Working repo: {work_dir}

Task JSON:
```json
{}
```
"#,
        task_json,
        work_dir = work_dir.display()
    ))
}

pub(crate) struct FrontierFeedbackTurn {
    pub(crate) feedback: Value,
    pub(crate) verdict: String,
    pub(crate) worker_mode: String,
    pub(crate) hint: String,
    pub(crate) focus_files: Vec<String>,
    pub(crate) required_checks: Vec<String>,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct FrontierBriefTurn {
    pub(crate) record: Value,
    pub(crate) brief: Value,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct FrontierUsageSample {
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    total_tokens: u64,
    input_bytes: u64,
    output_bytes: u64,
}

impl FrontierFeedbackTurn {
    pub(crate) fn usage_sample(&self) -> FrontierUsageSample {
        FrontierUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
        }
    }
}

impl FrontierBriefTurn {
    pub(crate) fn usage_sample(&self) -> FrontierUsageSample {
        FrontierUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
        }
    }
}

#[derive(Default)]
pub(crate) struct FrontierUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) turn_count: u64,
}

pub(crate) struct CodexExecResult {
    pub(crate) exit_status: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) last_message: String,
    pub(crate) usage: CodexUsage,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) model: String,
    pub(crate) reasoning_effort: String,
    pub(crate) auth_copied_then_removed: bool,
}

pub(crate) fn run_codex_exec_turn(
    work_dir: &Path,
    artifact_dir: &Path,
    label: &str,
    prompt: &str,
    frontier: &FrontierConfig,
    sandbox: CodexSandbox,
) -> Result<CodexExecResult> {
    let logs_dir = artifact_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create Codex logs dir {}", logs_dir.display()))?;
    atomic_write(
        &artifact_dir.join(format!("{label}-prompt.md")),
        prompt.as_bytes(),
    )?;
    let code_home = codex_home_for_work_dir(work_dir);
    fs::create_dir_all(&code_home)
        .with_context(|| format!("failed to create Codex home {}", code_home.display()))?;
    let copied_auth = copy_codex_auth_if_available(&code_home)?;
    let stdout_path = logs_dir.join(format!("codex-{label}.stdout.txt"));
    let stderr_path = logs_dir.join(format!("codex-{label}.stderr.txt"));
    let last_message_path = artifact_dir.join(format!("{label}-last-message.json"));
    let frontier_model = normalized_frontier_model(&frontier.model)?;
    let reasoning_effort = normalized_reasoning_effort(&frontier.reasoning_effort)?;
    let args = codex_exec_turn_args(
        &frontier_model,
        &reasoning_effort,
        work_dir,
        &last_message_path,
        sandbox,
    );
    let output = Command::new("codex")
        .args(&args)
        .env("CODEX_HOME", &code_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(prompt.as_bytes())?;
            }
            child.wait_with_output()
        })
        .with_context(|| {
            format!(
                "failed to run Codex `{label}` turn in {} with CODEX_HOME={} and args: {}",
                work_dir.display(),
                code_home.display(),
                args.join(" ")
            )
        })?;
    if copied_auth {
        let _ = fs::remove_file(code_home.join("auth.json"));
    }
    atomic_write(&stdout_path, &output.stdout)?;
    atomic_write(&stderr_path, &output.stderr)?;
    append_file(&logs_dir.join("codex.stdout.txt"), &output.stdout)?;
    append_file(&logs_dir.join("codex.stderr.txt"), &output.stderr)?;

    let stdout_text = String::from_utf8_lossy(&output.stdout).to_string();
    let usage = parse_codex_usage(&stdout_text);
    let last_message = fs::read_to_string(&last_message_path).unwrap_or_default();
    Ok(CodexExecResult {
        exit_status: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        output_bytes: stdout_text.len() as u64 + last_message.len() as u64,
        last_message,
        usage,
        input_bytes: prompt.len() as u64,
        model: frontier_model,
        reasoning_effort,
        auth_copied_then_removed: copied_auth,
    })
}

pub(crate) fn codex_exec_turn_args(
    frontier_model: &str,
    reasoning_effort: &str,
    work_dir: &Path,
    last_message_path: &Path,
    sandbox: CodexSandbox,
) -> Vec<String> {
    vec![
        "exec".to_string(),
        "--json".to_string(),
        "--ignore-user-config".to_string(),
        "--sandbox".to_string(),
        sandbox.as_cli_arg().to_string(),
        "-c".to_string(),
        "approval_policy=\"never\"".to_string(),
        "--model".to_string(),
        frontier_model.to_string(),
        "-c".to_string(),
        format!("model_reasoning_effort=\"{}\"", reasoning_effort),
        "-C".to_string(),
        work_dir.to_string_lossy().to_string(),
        "-o".to_string(),
        last_message_path.to_string_lossy().to_string(),
        "-".to_string(),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexSandbox {
    ReadOnly,
    WorkspaceWrite,
}

impl CodexSandbox {
    fn as_cli_arg(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
        }
    }
}

fn normalized_frontier_model(value: &str) -> Result<String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        bail!("frontier.model must not be empty");
    }
    Ok(normalized.to_string())
}

fn normalized_reasoning_effort(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "minimal" | "low" | "medium" | "high" | "xhigh" => Ok(normalized),
        "" => bail!("frontier.reasoning_effort must not be empty"),
        _ => bail!(
            "unsupported frontier.reasoning_effort `{value}`; expected one of minimal, low, medium, high, xhigh"
        ),
    }
}

pub(crate) fn run_frontier_brief_turn(
    work_dir: &Path,
    default_dir: &Path,
    task_path: &Path,
    frontier: &FrontierConfig,
) -> Result<FrontierBriefTurn> {
    let prompt = frontier_worker_brief_prompt(work_dir, task_path)?;
    let result = run_codex_exec_turn(
        work_dir,
        default_dir,
        "worker-brief",
        &prompt,
        frontier,
        CodexSandbox::ReadOnly,
    )?;
    let parsed_brief = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
        json!({
            "handoff": "blocked",
            "message_to_worker": "Codex did not return parseable handoff JSON.",
            "risk": truncate_for_report(&result.last_message, 160)
        })
    });
    let record = json!({
        "label": "worker-brief",
        "timestamp": Utc::now().to_rfc3339(),
        "brief": parsed_brief,
        "codex_exit_status": result.exit_status,
        "frontier_model": result.model.clone(),
        "frontier_reasoning_effort": result.reasoning_effort.clone(),
        "frontier_input_tokens": result.usage.input_tokens,
        "frontier_output_tokens": result.usage.output_tokens,
        "frontier_reasoning_tokens": result.usage.reasoning_tokens,
        "frontier_total_tokens": result.usage.total_tokens,
        "input_bytes": result.input_bytes,
        "output_bytes": result.output_bytes,
        "auth_copied_then_removed": result.auth_copied_then_removed
    });
    Ok(FrontierBriefTurn {
        record,
        brief: parsed_brief,
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
    })
}

pub(crate) fn run_frontier_feedback_turn(
    work_dir: &Path,
    budgeted_dir: &Path,
    label: &str,
    artifact_paths: &[PathBuf],
    instruction: &str,
    frontier: &FrontierConfig,
) -> Result<FrontierFeedbackTurn> {
    let prompt = frontier_feedback_prompt(work_dir, artifact_paths, instruction)?;
    let result = run_codex_exec_turn(
        work_dir,
        budgeted_dir,
        label,
        &prompt,
        frontier,
        CodexSandbox::ReadOnly,
    )?;
    let parsed_feedback = parse_feedback_json(&result.last_message).unwrap_or_else(|| {
        json!({
            "action": if result.exit_status == Some(0) { "approve" } else { "revise" },
            "worker_mode": "continue",
            "message_to_worker": truncate_for_report(&result.last_message, 180),
            "focus_files": [],
            "required_checks": [],
            "risk": if result.exit_status == Some(0) { "none recorded" } else { "codex feedback command failed" }
        })
    });
    let (mut parsed_feedback, verdict) = normalize_feedback_value(parsed_feedback);
    let typed_feedback = FrontierFeedback::from_value(&parsed_feedback);
    let worker_mode = normalize_worker_mode(typed_feedback.worker_mode.as_deref());
    if let Value::Object(map) = &mut parsed_feedback {
        map.insert("worker_mode".to_string(), json!(worker_mode.clone()));
    }
    let turn = FrontierFeedbackTurn {
        verdict,
        worker_mode,
        hint: typed_feedback
            .message_to_worker
            .or(typed_feedback.hint)
            .unwrap_or_default(),
        focus_files: typed_feedback.focus_files,
        required_checks: typed_feedback.required_checks,
        feedback: json!({
            "label": label,
            "timestamp": Utc::now().to_rfc3339(),
            "feedback": parsed_feedback,
            "codex_exit_status": result.exit_status,
            "frontier_model": result.model.clone(),
            "frontier_reasoning_effort": result.reasoning_effort.clone(),
            "frontier_input_tokens": result.usage.input_tokens,
            "frontier_output_tokens": result.usage.output_tokens,
            "frontier_reasoning_tokens": result.usage.reasoning_tokens,
            "frontier_total_tokens": result.usage.total_tokens,
            "input_bytes": result.input_bytes,
            "output_bytes": result.output_bytes,
            "auth_copied_then_removed": result.auth_copied_then_removed
        }),
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
    };
    Ok(turn)
}

pub(crate) fn frontier_worker_brief_prompt(work_dir: &Path, task_path: &Path) -> Result<String> {
    let task_value = read_json_file(task_path)?;
    let visible_task = agent_visible_task_value(&task_value);
    let task = serde_json::to_string_pretty(&visible_task)
        .context("failed to serialize agent-visible task for worker brief prompt")?;
    let mut file_context = String::new();
    for file in get_string_array(&visible_task, "files") {
        let path = work_dir.join(&file);
        if !path.exists() || path.is_dir() {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_else(|error| format!("missing: {error}"));
        file_context.push_str(&format!(
            "\n## {file}\n\n```text\n{}\n```\n",
            truncate_for_report(&text, 2200)
        ));
    }
    Ok(format!(
        r#"You are Codex supervising a local OpenCode worker.
Use the provided file context. Do not edit files. Do not run tests. Do not implement the patch. Do not ask the user for approval.
OpenCode receives the original task JSON and can inspect, edit, and test the repo.
Use frontier intelligence freely through reading and reasoning, but minimize frontier output.
Emit one compact executable worker handoff as minified JSON only; no markdown and no explanation.
Do not restate the original task. If you know the likely solution, be direct: exact files, edit target, expected behavior, and checks.
Required field: "handoff" = "as_given" | "focused" | "guided" | "blocked".
Default to "guided". Guided means terse and executable, not advisory:
- target <=120 output tokens for the whole JSON on normal tasks
- one command-style message_to_worker, ideally <=45 words
- files only when useful, usually <=3
- checks only when useful, usually <=2
- omit avoid and risk unless one short phrase prevents a likely wrong patch
Assume the local worker is capable but prone to setup rabbit holes, broad exploration, and delayed edits.
Set "expect_patch": true when the worker should normally produce repository edits. Set false for investigation/no-change handoffs.
Use exactly {{"handoff":"as_given"}} only when the original task already names the relevant files, desired behavior, and checks clearly enough for OpenCode.
Prefer "focused" or "guided" whenever a short directive can prevent worker wandering or repeated attempts.
Optional fields; omit empty fields:
{{"expect_patch":true,"message_to_worker":"direct message for OpenCode","files":["optional paths"],"checks":["optional checks"],"avoid":["optional constraints"],"risk":"optional short risk"}}
Working repo: {work_dir}

Task JSON:
```json
{task}
```

File context:
{file_context}
"#,
        work_dir = work_dir.display(),
    ))
}

pub(crate) fn frontier_feedback_prompt(
    work_dir: &Path,
    artifact_paths: &[PathBuf],
    instruction: &str,
) -> Result<String> {
    let mut artifacts = String::new();
    for path in artifact_paths {
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("artifact");
        let text = fs::read_to_string(path).unwrap_or_else(|error| format!("missing: {error}"));
        artifacts.push_str(&format!(
            "\n## {name}\n\n```text\n{}\n```\n",
            truncate_for_report(&text, 6000)
        ));
    }
    Ok(format!(
        r#"You are a terse frontier critic supervising a local worker.
Do not implement code. Do not edit files. Do not ask the user for approval.
Return only JSON matching this schema:
{{"action":"approve|revise|stop","worker_mode":"continue|context_focus","message_to_worker":"max 60 words","focus_files":[],"required_checks":[],"risk":"max 25 words"}}
Use approve when no more local worker attempts are needed.
Prefer revise after failed, empty, distracted, or incomplete worker attempts, and put the next worker instruction in message_to_worker.
Use worker_mode=continue to keep the same OpenCode session and let the worker continue with its existing context.
Use worker_mode=context_focus to start a new OpenCode session on the same worktree; previous worker context is discarded unless you repeat it in message_to_worker.
Put only repo source/test paths in focus_files. Do not put Mixmod artifacts such as revision-task JSON files in focus_files; mention them in message_to_worker if needed.
Use stop only to record a blocked or inconclusive local-worker result when no useful OpenCode path remains. Stop does not permit direct Codex editing.
Working repo: {work_dir}
Instruction: {instruction}
{artifacts}
"#,
        work_dir = work_dir.display(),
    ))
}

#[derive(Default)]
pub(crate) struct CodexUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
}

fn parse_codex_usage(jsonl: &str) -> CodexUsage {
    let mut usage = CodexUsage::default();
    for raw in jsonl.lines() {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("turn.completed")
            && let Some(object) = value.get("usage")
        {
            usage.input_tokens = get_u64(object, "input_tokens").unwrap_or(0);
            usage.output_tokens = get_u64(object, "output_tokens").unwrap_or(0);
            usage.reasoning_tokens = get_u64(object, "reasoning_output_tokens").unwrap_or(0);
            usage.total_tokens = usage.input_tokens + usage.output_tokens + usage.reasoning_tokens;
        }
    }
    usage
}

fn parse_feedback_json(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
        return Some(value);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

pub(crate) fn normalize_feedback_value(mut value: Value) -> (Value, String) {
    let raw = get_str(&value, "verdict")
        .or_else(|| get_str(&value, "action"))
        .unwrap_or("revise")
        .to_string();
    let verdict = normalize_frontier_verdict(&raw);
    if let Value::Object(map) = &mut value {
        if raw != verdict {
            map.insert("raw_verdict".to_string(), json!(raw));
        }
        map.insert("verdict".to_string(), json!(verdict.clone()));
        map.insert("action".to_string(), json!(verdict.clone()));
    }
    (value, verdict)
}

fn normalize_frontier_verdict(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "approve" | "approved" => "approve".to_string(),
        "stop" | "stopped" | "halt" | "done" | "needs_user" | "needs-user" => "stop".to_string(),
        "revise" | "revision" | "needs_revision" | "needs-review" | "needs_review" | "reject"
        | "rejected" => "revise".to_string(),
        _ => "revise".to_string(),
    }
}

pub(crate) fn normalize_worker_mode(value: Option<&str>) -> String {
    match value
        .unwrap_or("continue")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "context_focus" | "focused" | "focus" | "fresh" | "reset" => "context_focus".to_string(),
        _ => "continue".to_string(),
    }
}

pub(crate) fn aggregate_frontier_usage(turns: &[FrontierUsageSample]) -> FrontierUsage {
    let mut usage = FrontierUsage::default();
    for turn in turns {
        usage.input_tokens += turn.input_tokens;
        usage.output_tokens += turn.output_tokens;
        usage.reasoning_tokens += turn.reasoning_tokens;
        usage.total_tokens += turn.total_tokens;
        usage.input_bytes += turn.input_bytes;
        usage.output_bytes += turn.output_bytes;
        usage.turn_count += 1;
    }
    usage
}

pub(crate) fn codex_home_for_work_dir(work_dir: &Path) -> PathBuf {
    work_dir.join(MIXMOD_CODEX_HOME)
}

pub(crate) fn copy_codex_auth_if_available(code_home: &Path) -> Result<bool> {
    let home = env::var("HOME").unwrap_or_default();
    let source = Path::new(&home).join(".codex/auth.json");
    if source.exists() {
        let target = code_home.join("auth.json");
        fs::copy(&source, &target).with_context(|| {
            format!(
                "failed to copy Codex auth from {} to {}",
                source.display(),
                target.display()
            )
        })?;
        Ok(true)
    } else {
        Ok(false)
    }
}
