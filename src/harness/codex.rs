use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::{FrontierConfig, append_file, atomic_write, get_str, get_u64, state_layout};

/// Result of one Codex app-server turn.
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

/// Codex sandbox profile used for an app-server thread or turn.
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

/// Token usage reported by Codex app-server.
#[derive(Default)]
pub(crate) struct CodexUsage {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
}

/// Persistent Codex app-server process plus one active thread.
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
    /// Start a Codex app-server process and create one thread.
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
        let mut command = Command::new("codex");
        command
            .args(["app-server", "--listen", "stdio://"])
            .env("CODEX_HOME", &code_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let mut child = command.spawn().with_context(|| {
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
        #[cfg(unix)]
        signal_process_group(self.child.id(), SIGTERM);
        let _ = self.child.kill();
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        #[cfg(unix)]
        signal_process_group(self.child.id(), SIGKILL);
        let _ = self.child.try_wait();
        let _ = self.stderr_thread.take();
        if self.copied_auth {
            let _ = fs::remove_file(self.code_home.join("auth.json"));
        }
    }
}

#[cfg(unix)]
const SIGTERM: i32 = 15;

#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    if pid <= 1 {
        return;
    }
    unsafe {
        let _ = kill(-pid, signal);
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

pub(crate) fn codex_home_for_work_dir(work_dir: &Path) -> PathBuf {
    state_layout(work_dir).codex_home()
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
