use std::path::Path;

use anyhow::Result;

use crate::SupervisorConfig;
use crate::harness::codex::{CodexAppServer, CodexSandbox, CodexTurnResult};

/// Persistent Codex app-server session used by the supervisor loop.
pub(crate) struct SupervisorCodexSession {
    server: CodexAppServer,
}

impl SupervisorCodexSession {
    /// Start one Codex app-server process and supervisor thread for this run.
    pub(crate) fn start(work_dir: &Path, supervisor: &SupervisorConfig) -> Result<Self> {
        let sandbox = supervisor_codex_sandbox_from_env()?;
        let server = CodexAppServer::start(work_dir, supervisor, sandbox)?;
        Ok(Self { server })
    }

    /// Start a fresh read-only Codex app-server session for bounded review.
    pub(crate) fn start_spin_out_review(
        work_dir: &Path,
        supervisor: &SupervisorConfig,
    ) -> Result<Self> {
        let server = CodexAppServer::start(work_dir, supervisor, CodexSandbox::ReadOnly)?;
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

    /// Compact the active supervisor app-server thread.
    pub(crate) fn compact(&mut self, artifact_dir: &Path, label: &str) -> Result<CodexTurnResult> {
        self.server.compact_thread(artifact_dir, label)
    }
}

fn supervisor_codex_sandbox_from_env() -> Result<CodexSandbox> {
    CodexSandbox::from_env_var(
        "MIXMOD_CODEX_SUPERVISOR_SANDBOX",
        supervisor_default_codex_sandbox(),
    )
}

fn supervisor_default_codex_sandbox() -> CodexSandbox {
    CodexSandbox::WorkspaceWrite
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
            CodexSandbox::from_label("read-only").unwrap(),
            CodexSandbox::ReadOnly
        );
    }
}
