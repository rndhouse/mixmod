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

use crate::harness::{AgentBackend, AgentHarness, AgentOutput, AgentRequest};
use crate::{
    MixmodConfig, SupervisorConfig, append_file, atomic_write, get_str, get_u64, state_layout,
};

/// Result of one Codex app-server turn.
pub(crate) struct CodexTurnResult {
    pub(crate) exit_status: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) last_message: String,
    pub(crate) turn_status: Option<String>,
    pub(crate) error_info: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) usage: CodexUsage,
    pub(crate) input_bytes: u64,
    pub(crate) output_bytes: u64,
    pub(crate) model: String,
    pub(crate) reasoning_effort: String,
    pub(crate) auth_copied_then_removed: bool,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) token_usage_source: String,
    pub(crate) rollout_count: u64,
}

/// Codex sandbox profile used for an app-server thread or turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexSandbox {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl CodexSandbox {
    pub(crate) fn as_thread_arg(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    pub(crate) fn as_turn_policy(self, work_dir: &Path) -> Value {
        match self {
            Self::DangerFullAccess => json!({"type": "dangerFullAccess"}),
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

/// Token usage reported by Codex.
#[derive(Clone, Default)]
pub(crate) struct CodexUsage {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
}

/// Run one prompt through direct `codex exec`.
pub(crate) fn run_codex_exec_turn(
    work_dir: &Path,
    artifact_dir: &Path,
    label: &str,
    prompt: &str,
    supervisor: &SupervisorConfig,
    sandbox: CodexSandbox,
) -> Result<CodexTurnResult> {
    let logs_dir = artifact_dir.join("logs");
    fs::create_dir_all(&logs_dir).with_context(|| {
        format!(
            "failed to create Codex exec logs dir {}",
            logs_dir.display()
        )
    })?;
    atomic_write(
        &artifact_dir.join(format!("{label}-prompt.md")),
        prompt.as_bytes(),
    )?;

    let code_home = codex_home_for_work_dir(work_dir);
    if code_home.exists() {
        fs::remove_dir_all(&code_home)
            .with_context(|| format!("failed to reset Codex home {}", code_home.display()))?;
    }
    fs::create_dir_all(&code_home)
        .with_context(|| format!("failed to create Codex home {}", code_home.display()))?;
    let copied_auth = copy_codex_auth_if_available(&code_home)?;
    let model = normalized_supervisor_model(&supervisor.model)?;
    let reasoning_effort = normalized_reasoning_effort(&supervisor.reasoning_effort)?;
    let last_message_path = artifact_dir.join(format!("{label}-last-message.md"));
    let cd_arg = work_dir.to_string_lossy().to_string();
    let last_message_arg = last_message_path.to_string_lossy().to_string();
    let effort_config = format!("model_reasoning_effort=\"{reasoning_effort}\"");

    let mut child = Command::new("codex")
        .args(["--ask-for-approval", "never"])
        .arg("exec")
        .arg("--json")
        .args(["--model", model.as_str()])
        .args(["--sandbox", sandbox.as_thread_arg()])
        .args(["--cd", cd_arg.as_str()])
        .args(["--config", effort_config.as_str()])
        .args(["--output-last-message", last_message_arg.as_str()])
        .arg("-")
        .env("CODEX_HOME", &code_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to start codex exec in {} with CODEX_HOME={}",
                work_dir.display(),
                code_home.display()
            )
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("codex exec stdin was not available"))?
        .write_all(prompt.as_bytes())
        .context("failed to write codex exec prompt")?;
    let output = child.wait_with_output().context("codex exec failed")?;
    let stdout = output.stdout;
    let stderr = output.stderr;
    let exit_status = output.status.code();
    atomic_write(
        &logs_dir.join(format!("{label}.exec.stdout.jsonl")),
        &stdout,
    )?;
    atomic_write(&logs_dir.join(format!("{label}.exec.stderr.txt")), &stderr)?;

    let rollout_report = collect_codex_rollout_usage(&code_home, &logs_dir)?;
    let stdout_usage = codex_usage_from_jsonl(&stdout);
    let (usage, token_usage_source) = if rollout_report.usage.total_tokens > 0 {
        (
            rollout_report.usage.clone(),
            "codex_rollout_total_token_usage".to_string(),
        )
    } else if stdout_usage.total_tokens > 0 {
        (stdout_usage, "codex_exec_stdout_token_count".to_string())
    } else {
        (CodexUsage::default(), "unavailable".to_string())
    };

    if copied_auth {
        let _ = fs::remove_file(code_home.join("auth.json"));
    }

    let last_message = fs::read_to_string(&last_message_path).unwrap_or_default();
    Ok(CodexTurnResult {
        exit_status,
        stdout: stdout.clone(),
        stderr,
        output_bytes: stdout.len() as u64 + last_message.len() as u64,
        last_message,
        turn_status: exit_status.map(|status| {
            if status == 0 {
                "completed".to_string()
            } else {
                "failed".to_string()
            }
        }),
        error_info: None,
        error_message: None,
        usage,
        input_bytes: prompt.len() as u64,
        model,
        reasoning_effort,
        auth_copied_then_removed: copied_auth,
        thread_id: String::new(),
        turn_id: String::new(),
        token_usage_source,
        rollout_count: rollout_report.rollout_count,
    })
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

/// Codex app-server worker harness.
pub struct ShellCodexRunner {
    config: MixmodConfig,
    server: Mutex<Option<CodexAppServer>>,
}

impl ShellCodexRunner {
    /// Create a Codex worker runner from Mixmod configuration.
    pub fn new(config: MixmodConfig) -> Self {
        Self {
            config,
            server: Mutex::new(None),
        }
    }
}

impl AgentHarness for ShellCodexRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentOutput> {
        if request.require_local {
            bail!(
                "worker_backend=codex cannot satisfy --require-local because Codex app-server is not a local inference backend"
            );
        }

        let mut guard = self
            .server
            .lock()
            .map_err(|_| anyhow!("Codex worker server lock was poisoned"))?;
        if request.resume_session_id.is_none() {
            *guard = Some(CodexAppServer::start(
                &request.root,
                &self.config.codex_worker,
                CodexSandbox::WorkspaceWrite,
                None,
            )?);
        }

        let server = guard.as_mut().ok_or_else(|| {
            anyhow!(
                "Codex worker cannot resume session `{}` without an active in-process app-server",
                request.resume_session_id.as_deref().unwrap_or("unknown")
            )
        })?;
        if let Some(resume_session_id) = request.resume_session_id.as_deref()
            && server.thread_id != resume_session_id
        {
            bail!(
                "Codex worker can only resume the active app-server thread `{}` in this process, but Mixmod requested `{}`",
                server.thread_id,
                resume_session_id
            );
        }

        let result = server.run_turn(&request.out_dir, "codex-worker", &request.instruction)?;
        let success = result.exit_status == Some(0);
        let session_reused = request.resume_session_id.is_some();
        let thread_id = result.thread_id.clone();
        let turn_id = result.turn_id.clone();
        let model = result.model.clone();
        let reasoning_effort = result.reasoning_effort.clone();
        let model_arg = format!("{}:{}", model, reasoning_effort);
        let mut verification_notes = vec![
            "Codex worker ran through app-server with workspace-write sandbox.".to_string(),
            "Local inference verification is not applicable to the Codex worker backend."
                .to_string(),
        ];
        if result.auth_copied_then_removed {
            verification_notes.push(
                "Codex auth was copied into the Mixmod-scoped CODEX_HOME for the app-server process."
                    .to_string(),
            );
        }
        Ok(AgentOutput {
            backend: AgentBackend::Codex,
            command_for_metrics: vec![
                "codex".to_string(),
                "app-server".to_string(),
                "--listen".to_string(),
                "stdio://".to_string(),
            ],
            segments: vec![json!({
                "backend": "codex",
                "worker_mode": if session_reused { "continue" } else { "new" },
                "thread_id": thread_id.clone(),
                "turn_id": turn_id,
                "model": model.clone(),
                "reasoning_effort": reasoning_effort,
                "exit_status": result.exit_status,
                "success": success,
                "input_tokens": result.usage.input_tokens,
                "cached_input_tokens": result.usage.cached_input_tokens,
                "output_tokens": result.usage.output_tokens,
                "reasoning_tokens": result.usage.reasoning_tokens,
                "total_tokens": result.usage.total_tokens,
                "input_bytes": result.input_bytes,
                "output_bytes": result.output_bytes,
                "turn_status": result.turn_status,
                "error_info": result.error_info,
                "error_message": result.error_message
            })],
            exit_status: result.exit_status,
            success,
            stdout: result.last_message.into_bytes(),
            stderr: result.stderr,
            provider: Some("codex".to_string()),
            model: Some(model),
            model_arg: Some(model_arg),
            session_label: Some(request.session_id.clone()),
            session_id: Some(thread_id),
            resume_session_id: request.resume_session_id.clone(),
            session_reused,
            interrupted_by_supervisor: false,
            supervisor_control_action: None,
            supervisor_control_events: Vec::new(),
            timed_out: false,
            idle_timed_out: false,
            heartbeat_count: 0,
            require_local: false,
            local_inference_verified: false,
            gpu_activity_observed: false,
            backend_activity_observed: true,
            verification_notes,
        })
    }
}

impl CodexAppServer {
    /// Start a Codex app-server process and create one thread.
    pub(crate) fn start(
        work_dir: &Path,
        supervisor: &SupervisorConfig,
        sandbox: CodexSandbox,
        _tool_proxy_config: Option<&MixmodConfig>,
    ) -> Result<Self> {
        let code_home = codex_home_for_work_dir(work_dir);
        fs::create_dir_all(&code_home)
            .with_context(|| format!("failed to create Codex home {}", code_home.display()))?;
        let copied_auth = copy_codex_auth_if_available(&code_home)?;
        let model = normalized_supervisor_model(&supervisor.model)?;
        let reasoning_effort = normalized_reasoning_effort(&supervisor.reasoning_effort)?;
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

        let (exit_status, turn_status, error_info, error_message) = loop {
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
                            let completed_turn_status =
                                get_str(turn, "status").map(ToOwned::to_owned);
                            let completed_error_info = turn
                                .get("error")
                                .and_then(|error| get_str(error, "codexErrorInfo"))
                                .map(ToOwned::to_owned);
                            let completed_error_message = turn
                                .get("error")
                                .and_then(|error| get_str(error, "message"))
                                .map(ToOwned::to_owned);
                            if let Some(text) = final_agent_message_from_turn(turn) {
                                last_agent_message = text;
                            }
                            let status = match get_str(turn, "status") {
                                Some("completed") => Some(0),
                                _ => Some(1),
                            };
                            events.push(message);
                            break (
                                status,
                                completed_turn_status,
                                completed_error_info,
                                completed_error_message,
                            );
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
            turn_status,
            error_info,
            error_message,
            usage,
            input_bytes: prompt.len() as u64,
            model: self.model.clone(),
            reasoning_effort: self.reasoning_effort.clone(),
            auth_copied_then_removed: self.copied_auth,
            thread_id: self.thread_id.clone(),
            turn_id,
            token_usage_source: "codex_app_server_last_token_usage".to_string(),
            rollout_count: 0,
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

struct CodexRolloutReport {
    usage: CodexUsage,
    rollout_count: u64,
}

fn collect_codex_rollout_usage(code_home: &Path, logs_dir: &Path) -> Result<CodexRolloutReport> {
    let mut usage = CodexUsage::default();
    let mut rollout_count = 0_u64;
    let Some(sessions_dir) = code_home.join("sessions").canonicalize().ok() else {
        return Ok(CodexRolloutReport {
            usage,
            rollout_count,
        });
    };
    let target_root = logs_dir.join("codex-rollouts");
    for path in rollout_paths(&sessions_dir)? {
        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read Codex rollout {}", path.display()))?;
        if let Some(file_usage) = codex_usage_from_jsonl(&bytes).nonzero() {
            usage.add(&file_usage);
        }
        rollout_count += 1;
        let rel = path.strip_prefix(&sessions_dir).unwrap_or(&path);
        atomic_write(&target_root.join(rel), &bytes)?;
    }
    Ok(CodexRolloutReport {
        usage,
        rollout_count,
    })
}

fn rollout_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut stack = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in
            fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

pub(crate) fn codex_usage_from_jsonl(bytes: &[u8]) -> CodexUsage {
    let mut usage = CodexUsage::default();
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if let Some(next) = codex_usage_from_json_value(&value) {
            usage = next;
        }
    }
    usage
}

fn codex_usage_from_json_value(value: &Value) -> Option<CodexUsage> {
    let payload = value.get("payload");
    if get_str(value, "type") == Some("event_msg")
        && payload.and_then(|payload| get_str(payload, "type")) == Some("token_count")
    {
        return payload
            .and_then(|payload| payload.get("info"))
            .and_then(|info| info.get("total_token_usage"))
            .map(codex_usage_from_snake_breakdown);
    }
    if get_str(value, "type") == Some("token_count") {
        return value
            .get("info")
            .and_then(|info| info.get("total_token_usage"))
            .map(codex_usage_from_snake_breakdown);
    }
    if get_str(value, "method") == Some("thread/tokenUsage/updated") {
        return value
            .get("params")
            .and_then(|params| params.get("tokenUsage"))
            .and_then(|token_usage| token_usage.get("total").or_else(|| token_usage.get("last")))
            .map(codex_usage_from_breakdown);
    }
    None
}

fn codex_usage_from_snake_breakdown(value: &Value) -> CodexUsage {
    CodexUsage {
        input_tokens: get_u64(value, "input_tokens").unwrap_or(0),
        cached_input_tokens: get_u64(value, "cached_input_tokens").unwrap_or(0),
        output_tokens: get_u64(value, "output_tokens").unwrap_or(0),
        reasoning_tokens: get_u64(value, "reasoning_output_tokens").unwrap_or(0),
        total_tokens: get_u64(value, "total_tokens").unwrap_or_else(|| {
            get_u64(value, "input_tokens").unwrap_or(0)
                + get_u64(value, "output_tokens").unwrap_or(0)
                + get_u64(value, "reasoning_output_tokens").unwrap_or(0)
        }),
    }
}

impl CodexUsage {
    fn add(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.total_tokens += other.total_tokens;
    }

    fn nonzero(self) -> Option<Self> {
        if self.total_tokens > 0 {
            Some(self)
        } else {
            None
        }
    }
}

fn normalized_supervisor_model(value: &str) -> Result<String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        bail!("supervisor.model must not be empty");
    }
    Ok(normalized.to_string())
}

fn normalized_reasoning_effort(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "minimal" | "low" | "medium" | "high" | "xhigh" => Ok(normalized),
        "" => bail!("supervisor.reasoning_effort must not be empty"),
        _ => bail!(
            "unsupported supervisor.reasoning_effort `{value}`; expected one of minimal, low, medium, high, xhigh"
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
