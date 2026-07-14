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
    pub(super) stdout_events_path: Option<&'a Path>,
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
        config.stdout_events_path.map(Path::to_path_buf),
        config.stdout_bytes,
        Arc::clone(&config.last_output_at),
    );
    let stderr_thread = spawn_pipe_logger(
        stderr,
        config.stderr_path.to_path_buf(),
        None,
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
    events_path: Option<PathBuf>,
    counter: Arc<AtomicU64>,
    last_output_at: Arc<AtomicU64>,
) -> JoinHandle<Result<()>> {
    std::thread::spawn(move || {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        let mut events_file = match events_path.as_ref() {
            Some(events_path) => Some(
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(events_path)
                    .with_context(|| format!("failed to open {}", events_path.display()))?,
            ),
            None => None,
        };
        let mut pending_event_line = Vec::new();
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
            if let Some(events_file) = events_file.as_mut() {
                append_json_event_lines(
                    &mut pending_event_line,
                    &buffer[..read],
                    events_file,
                    events_path.as_deref().unwrap_or(&path),
                )?;
            }
            file.flush().ok();
            counter.fetch_add(read as u64, Ordering::Relaxed);
            last_output_at.store(now_millis(), Ordering::Relaxed);
        }
        if let Some(events_file) = events_file.as_mut()
            && !pending_event_line.is_empty()
        {
            write_json_event_line(
                &pending_event_line,
                events_file,
                events_path.as_deref().unwrap_or(&path),
            )?;
        }
        Ok(())
    })
}

fn append_json_event_lines(
    pending: &mut Vec<u8>,
    chunk: &[u8],
    file: &mut fs::File,
    path: &Path,
) -> Result<()> {
    pending.extend_from_slice(chunk);
    while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
        let line = pending.drain(..=index).collect::<Vec<_>>();
        write_json_event_line(&line, file, path)?;
    }
    Ok(())
}

fn write_json_event_line(line: &[u8], file: &mut fs::File, path: &Path) -> Result<()> {
    let line = String::from_utf8_lossy(line);
    let trimmed = line.trim();
    if trimmed.is_empty() || !trimmed.starts_with('{') {
        return Ok(());
    }
    let Ok(event) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Ok(());
    };
    if !event.is_object() {
        return Ok(());
    }
    let event = serde_json::to_string(&event)
        .with_context(|| format!("failed to serialize JSON event for {}", path.display()))?;
    file.write_all(event.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.flush().ok();
    Ok(())
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
