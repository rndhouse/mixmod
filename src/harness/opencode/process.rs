use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};

use crate::{atomic_write, shell_command};

use super::config::opencode_command;

pub(super) struct RunningOpenCodeProcess {
    pub(super) child: std::process::Child,
    pub(super) stdout_thread: JoinHandle<Result<()>>,
    pub(super) stderr_thread: JoinHandle<Result<()>>,
}

pub(super) struct SpawnOpenCodeProcess<'a> {
    pub(super) command: &'a str,
    pub(super) args: &'a [String],
    pub(super) root: &'a Path,
    pub(super) stdout_path: &'a Path,
    pub(super) stderr_path: &'a Path,
    pub(super) stdout_bytes: Arc<AtomicU64>,
    pub(super) stderr_bytes: Arc<AtomicU64>,
    pub(super) last_output_at: Arc<AtomicU64>,
}

pub(super) fn spawn_opencode_process(
    config: SpawnOpenCodeProcess<'_>,
) -> Result<RunningOpenCodeProcess> {
    let mut child = opencode_command(config.command, config.root)
        .args(config.args)
        .current_dir(config.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn OpenCode command `{}`", config.command))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture OpenCode stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture OpenCode stderr"))?;
    let stdout_thread = spawn_pipe_logger(
        stdout,
        config.stdout_path.to_path_buf(),
        config.stdout_bytes,
        Arc::clone(&config.last_output_at),
    );
    let stderr_thread = spawn_pipe_logger(
        stderr,
        config.stderr_path.to_path_buf(),
        config.stderr_bytes,
        config.last_output_at,
    );
    Ok(RunningOpenCodeProcess {
        child,
        stdout_thread,
        stderr_thread,
    })
}

fn spawn_pipe_logger<R: Read + Send + 'static>(
    mut reader: R,
    path: PathBuf,
    counter: Arc<AtomicU64>,
    last_output_at: Arc<AtomicU64>,
) -> JoinHandle<Result<()>> {
    std::thread::spawn(move || {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader
                .read(&mut buffer)
                .with_context(|| format!("failed to read pipe for {}", path.display()))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .with_context(|| format!("failed to write {}", path.display()))?;
            file.flush().ok();
            counter.fetch_add(read as u64, Ordering::Relaxed);
            last_output_at.store(now_millis(), Ordering::Relaxed);
        }
        Ok(())
    })
}

pub(super) fn join_pipe_logger(
    handle: JoinHandle<Result<()>>,
    label: &str,
    notes: &mut Vec<String>,
) {
    match handle.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => notes.push(format!("OpenCode {label} log streaming failed: {error}")),
        Err(_) => notes.push(format!("OpenCode {label} log streaming thread panicked")),
    }
}

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn run_optional_logged_command(
    command: &str,
    root: &Path,
    log_path: &Path,
) -> Option<String> {
    if command.trim().is_empty() {
        let _ = atomic_write(log_path, b"command not configured\n");
        return None;
    }
    let output = shell_command(command).current_dir(root).output();
    match output {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&format!("$ {command}\n"));
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text.push_str(&format!("\nexit_status={:?}\n", output.status.code()));
            let _ = atomic_write(log_path, text.as_bytes());
            output.status.success().then_some(text)
        }
        Err(error) => {
            let text = format!("failed to run `{command}`: {error}\n");
            let _ = atomic_write(log_path, text.as_bytes());
            None
        }
    }
}

pub(super) fn run_optional_command_text(command: &str, root: &Path) -> Option<String> {
    if command.trim().is_empty() {
        return None;
    }
    let output = shell_command(command).current_dir(root).output().ok()?;
    let mut text = String::new();
    text.push_str(&format!("$ {command}\n"));
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.push_str(&format!("\nexit_status={:?}\n", output.status.code()));
    output.status.success().then_some(text)
}
