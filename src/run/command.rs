use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Command;

pub(crate) fn shell_command(command: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }

    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
}

/// Put the active virtual environment's executable directory first in a child
/// process `PATH`.
///
/// Some agent launchers decorate `PATH` before Mixmod's inherited environment
/// reaches later helper processes. When a caller intentionally runs Mixmod
/// inside a virtual environment, child commands should resolve `python`,
/// `pytest`, and similar executables from that environment first.
pub(crate) fn prioritize_virtual_env_path(command: &mut Command) {
    let Some(venv) = env::var_os("VIRTUAL_ENV").filter(|value| !value.is_empty()) else {
        return;
    };
    let inherited_path = command_env(command, "PATH")
        .flatten()
        .or_else(|| env::var_os("PATH"));
    prioritize_virtual_env_path_from(command, &venv, inherited_path);
}

fn prioritize_virtual_env_path_from(
    command: &mut Command,
    venv: &OsStr,
    inherited_path: Option<OsString>,
) {
    let bin_dir = virtual_env_bin_dir(venv);
    let mut paths = vec![bin_dir.clone()];
    if let Some(path) = inherited_path {
        paths.extend(env::split_paths(&path).filter(|path| path != &bin_dir));
    }
    if let Ok(joined) = env::join_paths(paths) {
        command.env("VIRTUAL_ENV", venv).env("PATH", joined);
    }
}

fn virtual_env_bin_dir(venv: &OsStr) -> PathBuf {
    PathBuf::from(venv).join(if cfg!(windows) { "Scripts" } else { "bin" })
}

fn command_env(command: &Command, name: &str) -> Option<Option<OsString>> {
    command
        .get_envs()
        .find_map(|(key, value)| (key == OsStr::new(name)).then(|| value.map(OsString::from)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prioritize_virtual_env_path_moves_venv_bin_to_front() {
        let mut command = Command::new("tool");
        command.env("PATH", "/usr/bin:/tmp/agent-venv/bin:/bin");
        let inherited_path = command_env(&command, "PATH").flatten();

        prioritize_virtual_env_path_from(
            &mut command,
            OsStr::new("/tmp/agent-venv"),
            inherited_path,
        );

        let path = command_env(&command, "PATH")
            .flatten()
            .and_then(|value| value.into_string().ok())
            .unwrap();
        assert_eq!(path, "/tmp/agent-venv/bin:/usr/bin:/bin");
    }
}
