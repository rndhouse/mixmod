use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

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
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

#[derive(Debug)]
pub(crate) struct FrontierBriefTurn {
    pub(crate) record: Value,
    pub(crate) brief: Value,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

#[derive(Clone)]
pub(crate) struct FrontierUsageSample {
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    total_tokens: u64,
    cached_input_tokens: u64,
    input_bytes: u64,
    output_bytes: u64,
    thread_id: String,
    turn_id: String,
}

impl FrontierFeedbackTurn {
    pub(crate) fn usage_sample(&self) -> FrontierUsageSample {
        FrontierUsageSample {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
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
            cached_input_tokens: self.cached_input_tokens,
            input_bytes: self.input_bytes,
            output_bytes: self.output_bytes,
            thread_id: self.thread_id.clone(),
            turn_id: self.turn_id.clone(),
        }
    }
}

#[derive(Default)]
pub(crate) struct FrontierUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) turn_count: u64,
    pub(crate) thread_ids: Vec<String>,
    pub(crate) turn_ids: Vec<String>,
}

impl FrontierUsage {
    pub(crate) fn thread_count(&self) -> u64 {
        self.thread_ids
            .iter()
            .filter(|id| !id.is_empty())
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            .len() as u64
    }

    pub(crate) fn thread_reuse_count(&self) -> u64 {
        let observed_thread_ids = self.thread_ids.iter().filter(|id| !id.is_empty()).count() as u64;
        let thread_count = self.thread_count();
        observed_thread_ids.saturating_sub(thread_count)
    }

    pub(crate) fn session_reused(&self) -> bool {
        self.thread_reuse_count() > 0
    }
}

pub(crate) struct CodexTurnResult {
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
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexSandbox {
    ReadOnly,
    WorkspaceWrite,
}

impl CodexSandbox {
    pub(crate) fn as_thread_arg(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
        }
    }

    pub(crate) fn as_turn_policy(self, work_dir: &Path) -> Value {
        match self {
            Self::ReadOnly => json!({"type": "readOnly", "networkAccess": false}),
            Self::WorkspaceWrite => json!({
                "type": "workspaceWrite",
                "writableRoots": [work_dir.to_string_lossy().to_string()],
                "networkAccess": false,
                "excludeTmpdirEnvVar": false,
                "excludeSlashTmp": false
            }),
        }
    }
}

pub(crate) struct CodexAppServer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Arc<Mutex<Vec<u8>>>,
    stderr_thread: Option<JoinHandle<()>>,
    code_home: PathBuf,
    copied_auth: bool,
    next_request_id: u64,
    work_dir: PathBuf,
    sandbox: CodexSandbox,
    model: String,
    reasoning_effort: String,
    thread_id: String,
}

impl CodexAppServer {
    /// Start a Codex app-server process and create one supervisor thread.
    pub(crate) fn start(
        work_dir: &Path,
        frontier: &FrontierConfig,
        sandbox: CodexSandbox,
    ) -> Result<Self> {
        let code_home = codex_home_for_work_dir(work_dir);
        fs::create_dir_all(&code_home)
            .with_context(|| format!("failed to create Codex home {}", code_home.display()))?;
        let copied_auth = copy_codex_auth_if_available(&code_home)?;
        let model = normalized_frontier_model(&frontier.model)?;
        let reasoning_effort = normalized_reasoning_effort(&frontier.reasoning_effort)?;
        let mut child = Command::new("codex")
            .args(["app-server", "--listen", "stdio://"])
            .env("CODEX_HOME", &code_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "failed to start Codex app-server in {} with CODEX_HOME={}",
                    work_dir.display(),
                    code_home.display()
                )
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Codex app-server stdin was not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Codex app-server stdout was not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("Codex app-server stderr was not available"))?;
        let stderr_buffer = Arc::new(Mutex::new(Vec::new()));
        let stderr_thread = Some(spawn_stderr_collector(stderr, Arc::clone(&stderr_buffer)));
        let mut server = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr: stderr_buffer,
            stderr_thread,
            code_home,
            copied_auth,
            next_request_id: 1,
            work_dir: work_dir.to_path_buf(),
            sandbox,
            model,
            reasoning_effort,
            thread_id: String::new(),
        };
        server.initialize()?;
        server.start_thread()?;
        Ok(server)
    }

    /// Run one prompt as a new turn on the app-server thread.
    pub(crate) fn run_turn(
        &mut self,
        artifact_dir: &Path,
        label: &str,
        prompt: &str,
    ) -> Result<CodexTurnResult> {
        let logs_dir = artifact_dir.join("logs");
        fs::create_dir_all(&logs_dir)
            .with_context(|| format!("failed to create Codex logs dir {}", logs_dir.display()))?;
        atomic_write(
            &artifact_dir.join(format!("{label}-prompt.md")),
            prompt.as_bytes(),
        )?;

        let mut events = Vec::new();
        let response = self.request(
            "turn/start",
            json!({
                "threadId": self.thread_id,
                "input": [{
                    "type": "text",
                    "text": prompt,
                    "text_elements": []
                }],
                "cwd": self.work_dir.to_string_lossy().to_string(),
                "approvalPolicy": "never",
                "approvalsReviewer": "user",
                "sandboxPolicy": self.sandbox.as_turn_policy(&self.work_dir),
                "model": self.model.clone(),
                "effort": self.reasoning_effort.clone()
            }),
            &mut events,
        )?;
        let turn_id = response
            .get("turn")
            .and_then(|turn| get_str(turn, "id"))
            .ok_or_else(|| anyhow!("Codex app-server turn/start response did not include turn.id"))?
            .to_string();
        let mut last_agent_message = String::new();
        let mut delta_messages = BTreeMap::<String, String>::new();
        let mut usage = CodexUsage::default();

        let exit_status = loop {
            let message = self.read_message()?;
            if self.handle_server_request(&message)? {
                events.push(message);
                continue;
            }
            if let Some(method) = get_str(&message, "method") {
                let params = message.get("params").unwrap_or(&Value::Null);
                match method {
                    "item/agentMessage/delta"
                        if get_str(params, "threadId") == Some(self.thread_id.as_str())
                            && get_str(params, "turnId") == Some(turn_id.as_str()) =>
                    {
                        if let Some(item_id) = get_str(params, "itemId")
                            && let Some(delta) = get_str(params, "delta")
                        {
                            delta_messages
                                .entry(item_id.to_string())
                                .or_default()
                                .push_str(delta);
                        }
                    }
                    "item/completed"
                        if get_str(params, "threadId") == Some(self.thread_id.as_str())
                            && get_str(params, "turnId") == Some(turn_id.as_str()) =>
                    {
                        if let Some(text) = agent_message_text(params.get("item")) {
                            last_agent_message = text;
                        }
                    }
                    "thread/tokenUsage/updated"
                        if get_str(params, "threadId") == Some(self.thread_id.as_str())
                            && get_str(params, "turnId") == Some(turn_id.as_str()) =>
                    {
                        if let Some(last) =
                            params.get("tokenUsage").and_then(|value| value.get("last"))
                        {
                            usage = codex_usage_from_breakdown(last);
                        }
                    }
                    "turn/completed"
                        if get_str(params, "threadId") == Some(self.thread_id.as_str()) =>
                    {
                        if let Some(turn) = params.get("turn")
                            && get_str(turn, "id") == Some(turn_id.as_str())
                        {
                            if let Some(text) = final_agent_message_from_turn(turn) {
                                last_agent_message = text;
                            }
                            let status = match get_str(turn, "status") {
                                Some("completed") => Some(0),
                                _ => Some(1),
                            };
                            events.push(message);
                            break status;
                        }
                    }
                    _ => {}
                }
            }
            events.push(message);
        };

        if last_agent_message.is_empty()
            && let Some((_, text)) = delta_messages.iter().next_back()
        {
            last_agent_message = text.clone();
        }

        let event_log = jsonl_bytes(&events)?;
        let stderr = self.stderr_snapshot();
        atomic_write(&logs_dir.join(format!("codex-{label}.jsonl")), &event_log)?;
        atomic_write(&logs_dir.join(format!("codex-{label}.stderr.txt")), &stderr)?;
        append_file(&logs_dir.join("codex.stdout.txt"), &event_log)?;
        append_file(&logs_dir.join("codex.stderr.txt"), &stderr)?;
        atomic_write(
            &artifact_dir.join(format!("{label}-last-message.json")),
            last_agent_message.as_bytes(),
        )?;

        Ok(CodexTurnResult {
            exit_status,
            stdout: event_log.clone(),
            stderr,
            output_bytes: event_log.len() as u64 + last_agent_message.len() as u64,
            last_message: last_agent_message,
            usage,
            input_bytes: prompt.len() as u64,
            model: self.model.clone(),
            reasoning_effort: self.reasoning_effort.clone(),
            auth_copied_then_removed: self.copied_auth,
            thread_id: self.thread_id.clone(),
            turn_id,
        })
    }

    fn initialize(&mut self) -> Result<()> {
        let mut events = Vec::new();
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "mixmod",
                    "title": "Mixmod",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true,
                    "requestAttestation": false
                }
            }),
            &mut events,
        )?;
        self.send_notification("initialized", json!({}))?;
        Ok(())
    }

    fn start_thread(&mut self) -> Result<()> {
        let mut events = Vec::new();
        let response = self.request(
            "thread/start",
            json!({
                "model": self.model.clone(),
                "cwd": self.work_dir.to_string_lossy().to_string(),
                "approvalPolicy": "never",
                "approvalsReviewer": "user",
                "sandbox": self.sandbox.as_thread_arg(),
                "config": {
                    "model_reasoning_effort": self.reasoning_effort.clone()
                },
                "serviceName": "mixmod",
                "threadSource": "mixmod"
            }),
            &mut events,
        )?;
        self.thread_id = response
            .get("thread")
            .and_then(|thread| get_str(thread, "id"))
            .ok_or_else(|| {
                anyhow!("Codex app-server thread/start response did not include thread.id")
            })?
            .to_string();
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value, events: &mut Vec<Value>) -> Result<Value> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        let request = json!({
            "id": id,
            "method": method,
            "params": params
        });
        self.write_json(&request)?;
        loop {
            let message = self.read_message()?;
            if message.get("id").and_then(Value::as_u64) == Some(id)
                && (message.get("result").is_some() || message.get("error").is_some())
            {
                if let Some(error) = message.get("error") {
                    bail!("Codex app-server `{method}` request failed: {error}");
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            self.handle_server_request(&message)?;
            events.push(message);
        }
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        self.write_json(&json!({
            "method": method,
            "params": params
        }))
    }

    fn write_json(&mut self, value: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.stdin, value)
            .context("failed to write Codex app-server JSON-RPC message")?;
        self.stdin
            .write_all(b"\n")
            .context("failed to write Codex app-server JSON-RPC newline")?;
        self.stdin
            .flush()
            .context("failed to flush Codex app-server stdin")
    }

    fn read_message(&mut self) -> Result<Value> {
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .context("failed to read Codex app-server stdout")?;
        if read == 0 {
            bail!(
                "Codex app-server stdout closed unexpectedly; stderr: {}",
                String::from_utf8_lossy(&self.stderr_snapshot())
            );
        }
        serde_json::from_str(line.trim_end()).with_context(|| {
            format!(
                "failed to parse Codex app-server JSON-RPC line: {}",
                line.trim_end()
            )
        })
    }

    fn handle_server_request(&mut self, message: &Value) -> Result<bool> {
        let Some(id) = message.get("id").cloned() else {
            return Ok(false);
        };
        let Some(method) = get_str(message, "method") else {
            return Ok(false);
        };
        if message.get("result").is_some() || message.get("error").is_some() {
            return Ok(false);
        }
        let response = match method {
            "item/commandExecution/requestApproval" => {
                json!({"id": id, "result": {"decision": "decline"}})
            }
            "item/fileChange/requestApproval" => {
                json!({"id": id, "result": {"decision": "decline"}})
            }
            "execCommandApproval" | "applyPatchApproval" => {
                json!({"id": id, "result": {"decision": "denied"}})
            }
            "item/tool/requestUserInput" => {
                json!({"id": id, "result": {"answers": {}}})
            }
            "mcpServer/elicitation/request" => {
                json!({"id": id, "result": {"action": "cancel", "content": null, "_meta": null}})
            }
            "item/permissions/requestApproval" => {
                json!({"id": id, "result": {"permissions": {}, "scope": "turn", "strictAutoReview": true}})
            }
            _ => json!({
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Mixmod does not handle Codex app-server reverse request `{method}`")
                }
            }),
        };
        self.write_json(&response)?;
        Ok(true)
    }

    fn stderr_snapshot(&self) -> Vec<u8> {
        self.stderr
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

impl Drop for CodexAppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
        if self.copied_auth {
            let _ = fs::remove_file(self.code_home.join("auth.json"));
        }
    }
}

fn spawn_stderr_collector(mut stderr: ChildStderr, buffer: Arc<Mutex<Vec<u8>>>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut guard) = buffer.lock() {
                        guard.extend_from_slice(&chunk[..n]);
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn jsonl_bytes(values: &[Value]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for value in values {
        serde_json::to_writer(&mut bytes, value)
            .context("failed to serialize Codex app-server event")?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn agent_message_text(item: Option<&Value>) -> Option<String> {
    let item = item?;
    if get_str(item, "type") == Some("agentMessage") {
        get_str(item, "text").map(ToOwned::to_owned)
    } else {
        None
    }
}

fn final_agent_message_from_turn(turn: &Value) -> Option<String> {
    let items = turn.get("items").and_then(Value::as_array)?;
    let mut fallback = None;
    for item in items {
        if get_str(item, "type") != Some("agentMessage") {
            continue;
        }
        let Some(text) = get_str(item, "text") else {
            continue;
        };
        fallback = Some(text.to_string());
        if get_str(item, "phase") == Some("final_answer") {
            return Some(text.to_string());
        }
    }
    fallback
}

fn codex_usage_from_breakdown(value: &Value) -> CodexUsage {
    CodexUsage {
        input_tokens: get_u64(value, "inputTokens").unwrap_or(0),
        cached_input_tokens: get_u64(value, "cachedInputTokens").unwrap_or(0),
        output_tokens: get_u64(value, "outputTokens").unwrap_or(0),
        reasoning_tokens: get_u64(value, "reasoningOutputTokens").unwrap_or(0),
        total_tokens: get_u64(value, "totalTokens").unwrap_or_else(|| {
            get_u64(value, "inputTokens").unwrap_or(0)
                + get_u64(value, "outputTokens").unwrap_or(0)
                + get_u64(value, "reasoningOutputTokens").unwrap_or(0)
        }),
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
    let result = run_codex_app_server_turn(
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
    let thread_id = result.thread_id.clone();
    let turn_id = result.turn_id.clone();
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
        "frontier_cached_input_tokens": result.usage.cached_input_tokens,
        "input_bytes": result.input_bytes,
        "output_bytes": result.output_bytes,
        "auth_copied_then_removed": result.auth_copied_then_removed,
        "codex_app_server_thread_id": thread_id.clone(),
        "codex_app_server_turn_id": turn_id.clone()
    });
    Ok(FrontierBriefTurn {
        record,
        brief: parsed_brief,
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        cached_input_tokens: result.usage.cached_input_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
        thread_id,
        turn_id,
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
    let result = run_codex_app_server_turn(
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
    let thread_id = result.thread_id.clone();
    let turn_id = result.turn_id.clone();
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
            "frontier_cached_input_tokens": result.usage.cached_input_tokens,
            "input_bytes": result.input_bytes,
            "output_bytes": result.output_bytes,
            "auth_copied_then_removed": result.auth_copied_then_removed,
            "codex_app_server_thread_id": thread_id.clone(),
            "codex_app_server_turn_id": turn_id.clone()
        }),
        input_tokens: result.usage.input_tokens,
        output_tokens: result.usage.output_tokens,
        reasoning_tokens: result.usage.reasoning_tokens,
        total_tokens: result.usage.total_tokens,
        cached_input_tokens: result.usage.cached_input_tokens,
        input_bytes: result.input_bytes,
        output_bytes: result.output_bytes,
        thread_id,
        turn_id,
    };
    Ok(turn)
}

pub(crate) fn run_codex_app_server_turn(
    work_dir: &Path,
    artifact_dir: &Path,
    label: &str,
    prompt: &str,
    frontier: &FrontierConfig,
    sandbox: CodexSandbox,
) -> Result<CodexTurnResult> {
    let mut server = CodexAppServer::start(work_dir, frontier, sandbox)?;
    server.run_turn(artifact_dir, label, prompt)
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
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
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
        usage.cached_input_tokens += turn.cached_input_tokens;
        usage.input_bytes += turn.input_bytes;
        usage.output_bytes += turn.output_bytes;
        usage.turn_count += 1;
        usage.thread_ids.push(turn.thread_id.clone());
        usage.turn_ids.push(turn.turn_id.clone());
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
