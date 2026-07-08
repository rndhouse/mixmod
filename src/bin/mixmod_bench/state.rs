use std::env;
use std::path::PathBuf;

/// Return the base state directory used by benchmark worker services.
pub(crate) fn bench_worker_state_base() -> PathBuf {
    if let Some(path) = env::var_os("MIXMOD_BENCH_STATE_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(path).join("mixmod/bench-workers");
    }
    if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home).join(".local/state/mixmod/bench-workers");
    }
    env::temp_dir().join("mixmod/bench-workers")
}
