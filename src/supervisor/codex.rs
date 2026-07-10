use std::env;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::harness::codex::{CodexAppServer, CodexSandbox, CodexTurnResult};
use crate::{MixmodConfig, SupervisorConfig};

/// Persistent Codex app-server session used by the supervisor loop.
pub(crate) struct SupervisorCodexSession {
    server: CodexAppServer,
}

impl SupervisorCodexSession {
    /// Start one Codex app-server process and supervisor thread for this run.
    pub(crate) fn start(
        work_dir: &Path,
        supervisor: &SupervisorConfig,
        tool_proxy_config: Option<&MixmodConfig>,
    ) -> Result<Self> {
        let sandbox = supervisor_codex_sandbox_from_env()?;
        let server = CodexAppServer::start(work_dir, supervisor, sandbox, tool_proxy_config)?;
        Ok(Self { server })
    }

    /// Run one supervisor turn on the existing app-server thread.
    pub(crate) fn run_turn(
        &mut self,
        artifact_dir: &Path,
        label: &str,
        prompt: &str,
    ) -> Result<CodexTurnResult> {
        self.server.run_turn(artifact_dir, label, prompt)
    }
}

fn supervisor_codex_sandbox_from_env() -> Result<CodexSandbox> {
    match env::var("MIXMOD_CODEX_SUPERVISOR_SANDBOX") {
        Ok(value) => supervisor_codex_sandbox_from_value(&value),
        Err(env::VarError::NotPresent) => Ok(supervisor_default_codex_sandbox()),
        Err(error) => Err(error).context("failed to read MIXMOD_CODEX_SUPERVISOR_SANDBOX"),
    }
}

fn supervisor_default_codex_sandbox() -> CodexSandbox {
    CodexSandbox::WorkspaceWrite
}

fn supervisor_codex_sandbox_from_value(value: &str) -> Result<CodexSandbox> {
    match value {
        "read-only" => Ok(CodexSandbox::ReadOnly),
        "workspace-write" => Ok(CodexSandbox::WorkspaceWrite),
        "danger-full-access" => Ok(CodexSandbox::DangerFullAccess),
        _ => bail!(
            "unsupported MIXMOD_CODEX_SUPERVISOR_SANDBOX value `{value}`; expected read-only, workspace-write, or danger-full-access"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_codex_sandbox_defaults_to_workspace_write() {
        assert_eq!(
            supervisor_default_codex_sandbox(),
            CodexSandbox::WorkspaceWrite
        );
    }

    #[test]
    fn supervisor_codex_sandbox_env_value_still_allows_read_only_override() {
        assert_eq!(
            supervisor_codex_sandbox_from_value("read-only").unwrap(),
            CodexSandbox::ReadOnly
        );
    }
}
