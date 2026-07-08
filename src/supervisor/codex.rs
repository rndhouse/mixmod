use std::env;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::SupervisorConfig;
use crate::harness::codex::{CodexAppServer, CodexSandbox, CodexTurnResult};

pub(crate) fn run_codex_app_server_turn(
    work_dir: &Path,
    artifact_dir: &Path,
    label: &str,
    prompt: &str,
    supervisor: &SupervisorConfig,
    sandbox: CodexSandbox,
) -> Result<CodexTurnResult> {
    let mut server = CodexAppServer::start(work_dir, supervisor, sandbox)?;
    server.run_turn(artifact_dir, label, prompt)
}

pub(super) fn supervisor_codex_sandbox_from_env() -> Result<CodexSandbox> {
    match env::var("MIXMOD_CODEX_SUPERVISOR_SANDBOX") {
        Ok(value) if value == "read-only" => Ok(CodexSandbox::ReadOnly),
        Ok(value) if value == "workspace-write" => Ok(CodexSandbox::WorkspaceWrite),
        Ok(value) if value == "danger-full-access" => Ok(CodexSandbox::DangerFullAccess),
        Ok(value) => bail!(
            "unsupported MIXMOD_CODEX_SUPERVISOR_SANDBOX value `{value}`; expected read-only, workspace-write, or danger-full-access"
        ),
        Err(env::VarError::NotPresent) => Ok(CodexSandbox::ReadOnly),
        Err(error) => Err(error).context("failed to read MIXMOD_CODEX_SUPERVISOR_SANDBOX"),
    }
}
