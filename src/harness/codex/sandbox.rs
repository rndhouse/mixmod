use std::path::Path;

use serde_json::{Value, json};

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
