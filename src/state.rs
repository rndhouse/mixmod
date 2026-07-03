use std::env;
use std::path::{Path, PathBuf};

/// Environment variable used to override the Mixmod state root.
pub(crate) const MIXMOD_STATE_DIR_ENV: &str = "MIXMOD_STATE_DIR";

/// Centralized filesystem layout for one repository's Mixmod runtime state.
#[derive(Debug, Clone)]
pub(crate) struct StateLayout {
    state_root: PathBuf,
    project_dir: PathBuf,
}

impl StateLayout {
    /// Return the root directory shared by all Mixmod projects.
    pub(crate) fn state_root(&self) -> &Path {
        &self.state_root
    }

    /// Return the central state directory for this repository.
    pub(crate) fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// Return the generated Mixmod config path for this repository.
    pub(crate) fn config(&self) -> PathBuf {
        self.project_dir.join("config.toml")
    }

    /// Return the generated OpenCode config path for this repository.
    pub(crate) fn opencode_config(&self) -> PathBuf {
        self.project_dir.join("opencode.json")
    }

    /// Return the Codex home used by Mixmod for this repository.
    pub(crate) fn codex_home(&self) -> PathBuf {
        self.project_dir.join("codex-home")
    }

    /// Return the directory where prompt-derived task files are written.
    pub(crate) fn tasks(&self) -> PathBuf {
        self.project_dir.join("tasks")
    }

    /// Return the directory where public `mixmod exec` runs are written.
    pub(crate) fn runs(&self) -> PathBuf {
        self.project_dir.join("runs")
    }

    /// Return the directory where debug benchmark experiments are written.
    pub(crate) fn experiments(&self) -> PathBuf {
        self.project_dir.join("experiments")
    }

    /// Return the directory reserved for any future Mixmod backups.
    pub(crate) fn backups(&self) -> PathBuf {
        self.project_dir.join("backups")
    }
}

/// Return the central state layout for `repo_root`.
pub(crate) fn state_layout(repo_root: &Path) -> StateLayout {
    let state_root = state_root();
    let project_dir = state_root.join("projects").join(project_id(repo_root));
    StateLayout {
        state_root,
        project_dir,
    }
}

fn state_root() -> PathBuf {
    if let Some(path) = env::var_os(MIXMOD_STATE_DIR_ENV).filter(|value| !value.is_empty()) {
        return absolute_path(PathBuf::from(path));
    }

    #[cfg(test)]
    {
        env::temp_dir().join("mixmod-test-state")
    }

    #[cfg(not(test))]
    {
        if let Some(path) = env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(path).join("mixmod");
        }
        if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(home).join(".local/state/mixmod");
        }
        env::temp_dir().join("mixmod")
    }
}

fn project_id(repo_root: &Path) -> String {
    let canonical = repo_root
        .canonicalize()
        .unwrap_or_else(|_| absolute_path(repo_root.to_path_buf()));
    let name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .map(sanitize_project_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "root".to_string());
    let hash = fnv1a64(canonical.to_string_lossy().as_bytes());
    format!("{name}-{hash:016x}")
}

fn sanitize_project_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
}
