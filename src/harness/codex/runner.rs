use std::sync::Mutex;

use anyhow::{Result, anyhow, bail};
use serde_json::json;

use super::app_server::CodexAppServer;
use super::sandbox::CodexSandbox;
use crate::MixmodConfig;
use crate::harness::{AgentBackend, AgentHarness, AgentOutput, AgentRequest};

const CODEX_WORKER_SANDBOX_ENV: &str = "MIXMOD_CODEX_WORKER_SANDBOX";

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
            let sandbox = codex_worker_sandbox_from_env()?;
            *guard = Some(CodexAppServer::start(
                &request.root,
                &self.config.codex_worker,
                sandbox,
            )?);
        }

        let server = guard.as_mut().ok_or_else(|| {
            anyhow!(
                "Codex worker cannot resume session `{}` without an active in-process app-server",
                request.resume_session_id.as_deref().unwrap_or("unknown")
            )
        })?;
        if let Some(resume_session_id) = request.resume_session_id.as_deref()
            && server.thread_id() != resume_session_id
        {
            bail!(
                "Codex worker can only resume the active app-server thread `{}` in this process, but Mixmod requested `{}`",
                server.thread_id(),
                resume_session_id
            );
        }

        let sandbox = server.sandbox();
        let result = server.run_turn(&request.out_dir, "codex-worker", &request.instruction)?;
        let success = result.exit_status == Some(0);
        let session_reused = request.resume_session_id.is_some();
        let thread_id = result.thread_id.clone();
        let turn_id = result.turn_id.clone();
        let model = result.model.clone();
        let reasoning_effort = result.reasoning_effort.clone();
        let model_arg = format!("{}:{}", model, reasoning_effort);
        let mut verification_notes = vec![
            format!(
                "Codex worker ran through app-server with {} sandbox.",
                sandbox.as_thread_arg()
            ),
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
                "sandbox": sandbox.as_thread_arg(),
                "token_usage_source": result.token_usage_source.clone(),
                "token_usage_scope": result.token_usage_scope.clone(),
                "token_usage_comparable": result.token_usage_comparable,
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

fn codex_worker_sandbox_from_env() -> Result<CodexSandbox> {
    CodexSandbox::from_env_var(CODEX_WORKER_SANDBOX_ENV, CodexSandbox::WorkspaceWrite)
}
