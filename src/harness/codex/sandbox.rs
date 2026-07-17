use std::env;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// Codex sandbox profile used for an app-server thread or turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexSandbox {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl CodexSandbox {
    /// Parse a Codex sandbox label from configuration or environment.
    pub(crate) fn from_label(value: &str) -> Result<Self> {
        match value {
            "read-only" => Ok(Self::ReadOnly),
            "workspace-write" => Ok(Self::WorkspaceWrite),
            "danger-full-access" => Ok(Self::DangerFullAccess),
            _ => bail!(
                "unsupported Codex sandbox value `{value}`; expected read-only, workspace-write, or danger-full-access"
            ),
        }
    }

    /// Read a Codex sandbox label from an environment variable.
    pub(crate) fn from_env_var(name: &str, default: Self) -> Result<Self> {
        match env::var(name) {
            Ok(value) => Self::from_label(&value),
            Err(env::VarError::NotPresent) => Ok(default),
            Err(error) => Err(error).with_context(|| format!("failed to read {name}")),
        }
    }

    /// Return the CLI/app-server thread argument for this sandbox.
    pub(crate) fn as_thread_arg(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    /// Return the app-server turn policy for this sandbox.
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
