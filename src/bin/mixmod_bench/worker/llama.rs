use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::mixmod_bench::process::{process_alive, signal_process};
use crate::mixmod_bench::state::bench_worker_state_base;

const DEFAULT_WORKER_NAME: &str = "deepswe-qwen36-llama";
const DEFAULT_HF_REPO: &str = "unsloth/Qwen3.6-27B-MTP-GGUF:Q4_K_M";
const DEFAULT_MODEL_ALIAS: &str = "qwen/qwen3.6-27b";
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_CLIENT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_CTX: u32 = 32768;
const DEFAULT_READY_TIMEOUT_SECS: u64 = 900;
const DEFAULT_READY_INTERVAL_SECS: u64 = 2;
const DEFAULT_TEARDOWN_TIMEOUT_SECS: u64 = 30;

/// Persisted state for the llama.cpp benchmark worker.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct LlamaWorkerState {
    /// Whether this Mixmod helper started the process.
    managed: bool,
    /// Server process id when the worker is managed.
    pid: Option<u32>,
    /// Human-readable lifecycle status.
    status: String,
    /// OpenAI-compatible API base URL for OpenCode.
    base_url: String,
    /// Readiness endpoint checked by the setup command.
    ready_url: String,
    /// Path to the llama-server log file.
    log: String,
    /// Path to the command audit log.
    command_log: String,
    /// UTC timestamp for the current lifecycle record.
    started_at: String,
    /// Model name exposed to OpenCode.
    model: String,
    /// Hugging Face repo/spec passed to llama-server.
    hf_repo: String,
    /// Host configured for llama-server.
    configured_host: String,
    /// Port configured for llama-server.
    configured_port: u16,
    /// Context size configured for llama-server.
    ctx_size: u32,
}

#[derive(Debug)]
struct LlamaConfig {
    worker_name: String,
    state_dir: PathBuf,
    state_env: PathBuf,
    state_json: PathBuf,
    log_path: PathBuf,
    command_path: PathBuf,
    hf_repo: String,
    alias: String,
    host: String,
    port: u16,
    ctx: u32,
    base_url: String,
    ready_url: String,
    ready_timeout: Duration,
    ready_interval: Duration,
    teardown_timeout: Duration,
    script_path: PathBuf,
}

/// Start or reuse the llama.cpp OpenAI-compatible worker.
pub(crate) fn setup() -> Result<()> {
    let config = LlamaConfig::from_env()?;
    fs::create_dir_all(&config.state_dir)
        .with_context(|| format!("failed to create {}", config.state_dir.display()))?;

    let old_state = read_state(&config.state_json).ok();
    if ready_check(&config.ready_url) {
        let (managed, pid) = old_state
            .as_ref()
            .filter(|state| {
                state.managed
                    && state.base_url == config.base_url
                    && state.pid.is_some_and(process_alive)
            })
            .map(|state| (true, state.pid))
            .unwrap_or((false, None));
        let started_at = now_rfc3339();
        write_command_log(&config, &started_at)?;
        write_state(&config, managed, pid, "ready", &started_at)?;
        println!(
            "SETUP_READY reused=true managed={} base_url={} state={} log={}",
            managed,
            config.base_url,
            config.state_json.display(),
            config.log_path.display()
        );
        return Ok(());
    }

    let old_pid = old_state
        .as_ref()
        .filter(|state| state.managed && state.base_url == config.base_url)
        .and_then(|state| state.pid)
        .filter(|pid| process_alive(*pid));
    let (pid, started_at) = if let Some(pid) = old_pid {
        let started_at = old_state
            .as_ref()
            .map(|state| state.started_at.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(now_rfc3339);
        (pid, started_at)
    } else {
        let started_at = now_rfc3339();
        write_command_log(&config, &started_at)?;
        let pid = spawn_llama_server(&config)?;
        write_state(&config, true, Some(pid), "starting", &started_at)?;
        println!(
            "SETUP_START pid={} base_url={} log={}",
            pid,
            config.base_url,
            config.log_path.display()
        );
        (pid, started_at)
    };

    let deadline = Instant::now() + config.ready_timeout;
    while Instant::now() < deadline {
        if ready_check(&config.ready_url) {
            write_state(&config, true, Some(pid), "ready", &started_at)?;
            println!(
                "SETUP_READY reused=false managed=true pid={} base_url={} state={} log={}",
                pid,
                config.base_url,
                config.state_json.display(),
                config.log_path.display()
            );
            return Ok(());
        }
        if !process_alive(pid) {
            write_state(&config, true, Some(pid), "exited_before_ready", &started_at)?;
            eprintln!(
                "SETUP_FAILED pid={} exited_before_ready log={}",
                pid,
                config.log_path.display()
            );
            print_tail(&config.log_path, 40);
            bail!("llama-server exited before readiness");
        }
        thread::sleep(config.ready_interval);
    }

    write_state(&config, true, Some(pid), "timeout", &started_at)?;
    eprintln!(
        "SETUP_FAILED timeout={}s pid={} base_url={} log={}",
        config.ready_timeout.as_secs(),
        pid,
        config.base_url,
        config.log_path.display()
    );
    print_tail(&config.log_path, 40);
    bail!("llama-server did not become ready");
}

/// Stop the llama.cpp worker if this helper started it.
pub(crate) fn teardown() -> Result<()> {
    let config = LlamaConfig::from_env()?;
    if !config.state_json.exists() {
        println!(
            "TEARDOWN_SKIP reason=no_state state={}",
            config.state_json.display()
        );
        return Ok(());
    }
    let mut state = read_state(&config.state_json)?;
    let log_path = state.log.clone();
    let Some(pid) = state.pid else {
        state.status = "external_or_unmanaged".to_string();
        write_state_json(&config.state_json, &state)?;
        println!("TEARDOWN_SKIP reason=unmanaged pid=none log={log_path}");
        return Ok(());
    };
    if !state.managed {
        state.status = "external_or_unmanaged".to_string();
        write_state_json(&config.state_json, &state)?;
        println!("TEARDOWN_SKIP reason=unmanaged pid={pid} log={log_path}");
        return Ok(());
    }
    if !process_alive(pid) {
        state.status = "already_stopped".to_string();
        write_state_json(&config.state_json, &state)?;
        println!("TEARDOWN_DONE already_stopped pid={pid} log={log_path}");
        return Ok(());
    }

    signal_process(pid, "TERM");
    let deadline = Instant::now() + config.teardown_timeout;
    while Instant::now() < deadline {
        if !process_alive(pid) {
            state.status = "stopped".to_string();
            write_state_json(&config.state_json, &state)?;
            println!("TEARDOWN_DONE pid={pid} log={log_path}");
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }

    signal_process(pid, "KILL");
    thread::sleep(Duration::from_secs(1));
    if process_alive(pid) {
        state.status = "kill_failed".to_string();
        write_state_json(&config.state_json, &state)?;
        eprintln!("TEARDOWN_FAILED pid={pid} log={log_path}");
        bail!("failed to stop llama-server pid {pid}");
    }

    state.status = "killed".to_string();
    write_state_json(&config.state_json, &state)?;
    println!("TEARDOWN_DONE killed=true pid={pid} log={log_path}");
    Ok(())
}

/// Run a benchmark command with the llama.cpp worker available.
pub(crate) fn run_with_worker(command: Vec<String>) -> Result<()> {
    if command.is_empty() {
        bail!("usage: mixmod-bench worker run-with-llama -- <command> [args...]");
    }
    setup()?;
    let config = LlamaConfig::from_env()?;
    let state = read_state(&config.state_json)?;
    println!(
        "BENCH_START worker_base_url={} command={}",
        state.base_url,
        shell_join(&command)
    );

    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .env("MIXMOD_OPENCODE_BASE_URL", &state.base_url)
        .spawn()
        .with_context(|| format!("failed to start {}", command[0]))?;
    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {}", command[0]))?;
    let code = status.code().unwrap_or(1);
    println!("BENCH_DONE exit={code}");

    if env::var("MIXMOD_KEEP_LLAMA_WORKER").unwrap_or_default() == "1" {
        println!("TEARDOWN_SKIP reason=keep_worker");
    } else {
        teardown().ok();
    }

    if status.success() {
        Ok(())
    } else {
        std::process::exit(code);
    }
}

impl LlamaConfig {
    fn from_env() -> Result<Self> {
        let worker_name = env_string("MIXMOD_LLAMA_WORKER_NAME")
            .unwrap_or_else(|| DEFAULT_WORKER_NAME.to_string());
        let state_dir = env_path("MIXMOD_LLAMA_WORKER_STATE_DIR")
            .unwrap_or_else(|| bench_worker_state_base().join(&worker_name));
        let state_env = state_dir.join("worker.env");
        let state_json = state_dir.join("worker.json");
        let log_path = env_path("MIXMOD_LLAMA_WORKER_LOG")
            .unwrap_or_else(|| state_dir.join("llama-server.log"));
        let command_path = state_dir.join("llama-server.command.txt");
        let hf_repo = env_string("LLAMA_SERVER_HF_REPO")
            .or_else(|| env_string("MIXMOD_LLAMA_HF_REPO"))
            .unwrap_or_else(|| DEFAULT_HF_REPO.to_string());
        let alias = env_string("LLAMA_SERVER_ALIAS")
            .or_else(|| env_string("MIXMOD_LLAMA_MODEL_ALIAS"))
            .unwrap_or_else(|| DEFAULT_MODEL_ALIAS.to_string());
        let host = env_string("LLAMA_SERVER_HOST")
            .or_else(|| env_string("MIXMOD_LLAMA_HOST"))
            .unwrap_or_else(|| DEFAULT_HOST.to_string());
        let port = env_u16("LLAMA_SERVER_PORT")
            .or_else(|| env_u16("MIXMOD_LLAMA_PORT"))
            .unwrap_or(DEFAULT_PORT);
        let ctx = env_u32("LLAMA_SERVER_CTX")
            .or_else(|| env_u32("MIXMOD_LLAMA_CTX_SIZE"))
            .unwrap_or(DEFAULT_CTX);
        let client_host = env_string("LLAMA_SERVER_CLIENT_HOST")
            .unwrap_or_else(|| DEFAULT_CLIENT_HOST.to_string());
        let base_url = env_string("MIXMOD_OPENCODE_BASE_URL")
            .unwrap_or_else(|| format!("http://{client_host}:{port}/v1"));
        let ready_url =
            env_string("MIXMOD_LLAMA_READY_URL").unwrap_or_else(|| format!("{base_url}/models"));
        let ready_timeout = Duration::from_secs(
            env_u64("MIXMOD_LLAMA_READY_TIMEOUT_SECONDS").unwrap_or(DEFAULT_READY_TIMEOUT_SECS),
        );
        let ready_interval = Duration::from_secs(
            env_u64("MIXMOD_LLAMA_READY_INTERVAL_SECONDS").unwrap_or(DEFAULT_READY_INTERVAL_SECS),
        );
        let teardown_timeout = Duration::from_secs(
            env_u64("MIXMOD_LLAMA_TEARDOWN_TIMEOUT_SECONDS")
                .unwrap_or(DEFAULT_TEARDOWN_TIMEOUT_SECS),
        );
        let script_path = llama_server_script_path()?;
        Ok(Self {
            worker_name,
            state_dir,
            state_env,
            state_json,
            log_path,
            command_path,
            hf_repo,
            alias,
            host,
            port,
            ctx,
            base_url,
            ready_url,
            ready_timeout,
            ready_interval,
            teardown_timeout,
            script_path,
        })
    }
}

fn spawn_llama_server(config: &LlamaConfig) -> Result<u32> {
    if let Some(parent) = config.log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    File::create(&config.log_path)
        .with_context(|| format!("failed to truncate {}", config.log_path.display()))?;
    let stdout = File::options()
        .append(true)
        .open(&config.log_path)
        .with_context(|| format!("failed to open {}", config.log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to clone {}", config.log_path.display()))?;
    let child = Command::new("bash")
        .arg(&config.script_path)
        .env("LLAMA_SERVER_HF_REPO", &config.hf_repo)
        .env("LLAMA_SERVER_ALIAS", &config.alias)
        .env("LLAMA_SERVER_HOST", &config.host)
        .env("LLAMA_SERVER_PORT", config.port.to_string())
        .env("LLAMA_SERVER_CTX", config.ctx.to_string())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| {
            format!(
                "failed to start llama-server command {}",
                config.script_path.display()
            )
        })?;
    Ok(child.id())
}

fn write_state(
    config: &LlamaConfig,
    managed: bool,
    pid: Option<u32>,
    status: &str,
    started_at: &str,
) -> Result<()> {
    let state = LlamaWorkerState {
        managed,
        pid,
        status: status.to_string(),
        base_url: config.base_url.clone(),
        ready_url: config.ready_url.clone(),
        log: config.log_path.to_string_lossy().into_owned(),
        command_log: config.command_path.to_string_lossy().into_owned(),
        started_at: started_at.to_string(),
        model: config.alias.clone(),
        hf_repo: config.hf_repo.clone(),
        configured_host: config.host.clone(),
        configured_port: config.port,
        ctx_size: config.ctx,
    };
    write_state_env(config, &state)?;
    write_state_json(&config.state_json, &state)
}

fn write_state_env(config: &LlamaConfig, state: &LlamaWorkerState) -> Result<()> {
    if let Some(parent) = config.state_env.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let tmp = config.state_env.with_extension("env.tmp");
    let mut file =
        File::create(&tmp).with_context(|| format!("failed to write {}", tmp.display()))?;
    writeln!(file, "LLAMA_WORKER_MANAGED={}", state.managed)?;
    writeln!(
        file,
        "LLAMA_WORKER_PID={}",
        state.pid.map(|pid| pid.to_string()).unwrap_or_default()
    )?;
    writeln!(file, "LLAMA_WORKER_STATUS={}", shell_quote(&state.status))?;
    writeln!(
        file,
        "LLAMA_WORKER_BASE_URL={}",
        shell_quote(&state.base_url)
    )?;
    writeln!(
        file,
        "LLAMA_WORKER_READY_URL={}",
        shell_quote(&state.ready_url)
    )?;
    writeln!(file, "LLAMA_WORKER_LOG={}", shell_quote(&state.log))?;
    writeln!(
        file,
        "LLAMA_WORKER_COMMAND_LOG={}",
        shell_quote(&state.command_log)
    )?;
    writeln!(
        file,
        "LLAMA_WORKER_STARTED_AT={}",
        shell_quote(&state.started_at)
    )?;
    writeln!(file, "LLAMA_WORKER_MODEL={}", shell_quote(&state.model))?;
    fs::rename(&tmp, &config.state_env)
        .with_context(|| format!("failed to move {}", config.state_env.display()))
}

fn write_state_json(path: &Path, state: &LlamaWorkerState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(state).context("failed to serialize worker state")?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, [bytes, b"\n".to_vec()].concat())
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to move {}", path.display()))
}

fn read_state(path: &Path) -> Result<LlamaWorkerState> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_command_log(config: &LlamaConfig, started_at: &str) -> Result<()> {
    if let Some(parent) = config.command_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut file = File::create(&config.command_path)
        .with_context(|| format!("failed to write {}", config.command_path.display()))?;
    writeln!(file, "worker_name={}", config.worker_name)?;
    writeln!(file, "started_at={started_at}")?;
    writeln!(file, "base_url={}", config.base_url)?;
    writeln!(file, "ready_url={}", config.ready_url)?;
    writeln!(file, "log={}", config.log_path.display())?;
    writeln!(file, "LLAMA_SERVER_HF_REPO={}", config.hf_repo)?;
    writeln!(file, "LLAMA_SERVER_ALIAS={}", config.alias)?;
    writeln!(file, "LLAMA_SERVER_HOST={}", config.host)?;
    writeln!(file, "LLAMA_SERVER_PORT={}", config.port)?;
    writeln!(file, "LLAMA_SERVER_CTX={}", config.ctx)?;
    writeln!(
        file,
        "LLAMA_SERVER_EXTRA_ARGS={}",
        env::var("LLAMA_SERVER_EXTRA_ARGS").unwrap_or_default()
    )?;
    writeln!(file, "command={}", shell_join(&server_command(config)))?;
    Ok(())
}

fn server_command(config: &LlamaConfig) -> Vec<String> {
    vec![
        "bash".to_string(),
        config.script_path.to_string_lossy().into_owned(),
    ]
}

fn ready_check(url: &str) -> bool {
    simple_http_get_status(url)
        .map(|status| (200..300).contains(&status))
        .unwrap_or(false)
}

fn simple_http_get_status(url: &str) -> Result<u16> {
    let parsed = ParsedHttpUrl::parse(url)?;
    let mut stream = TcpStream::connect((parsed.host.as_str(), parsed.port))
        .with_context(|| format!("failed to connect to {}", parsed.host_port()))?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(3))).ok();
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        parsed.path,
        parsed.host_port()
    );
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok();
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("invalid HTTP response from {}", parsed.host_port()))?;
    Ok(status)
}

struct ParsedHttpUrl {
    host: String,
    port: u16,
    path: String,
}

impl ParsedHttpUrl {
    fn parse(value: &str) -> Result<Self> {
        let rest = value
            .strip_prefix("http://")
            .ok_or_else(|| anyhow!("only http:// readiness URLs are supported: {value}"))?;
        let (authority, path) = rest
            .split_once('/')
            .map(|(authority, path)| (authority, format!("/{path}")))
            .unwrap_or((rest, "/".to_string()));
        let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
            let parsed_port = port
                .parse::<u16>()
                .with_context(|| format!("invalid port in {value}"))?;
            (host.to_string(), parsed_port)
        } else {
            (authority.to_string(), 80)
        };
        if host.is_empty() {
            bail!("missing host in readiness URL {value}");
        }
        Ok(Self { host, port, path })
    }

    fn host_port(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn llama_server_script_path() -> Result<PathBuf> {
    if let Some(path) = env_path("MIXMOD_LLAMA_SERVER_SCRIPT") {
        return Ok(path);
    }
    let cwd_candidate = env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("scripts/llama_server_qwen36.sh");
    if cwd_candidate.exists() {
        return Ok(cwd_candidate);
    }
    let manifest_candidate =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/llama_server_qwen36.sh");
    if manifest_candidate.exists() {
        return Ok(manifest_candidate);
    }
    bail!("failed to locate scripts/llama_server_qwen36.sh")
}

fn print_tail(path: &Path, lines: usize) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let tail = text.lines().rev().take(lines).collect::<Vec<_>>();
    for line in tail.into_iter().rev() {
        eprintln!("{line}");
    }
}

fn env_string(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.is_empty())
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn env_u64(key: &str) -> Option<u64> {
    env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
}

fn env_u32(key: &str) -> Option<u32> {
    env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
}

fn env_u16(key: &str) -> Option<u16> {
    env::var(key)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
}

fn now_rfc3339() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '='))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[allow(dead_code)]
fn os_string(value: &str) -> OsString {
    OsString::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_url_with_path() {
        let parsed = ParsedHttpUrl::parse("http://127.0.0.1:8080/v1/models").unwrap();
        assert_eq!(parsed.host, "127.0.0.1");
        assert_eq!(parsed.port, 8080);
        assert_eq!(parsed.path, "/v1/models");
    }

    #[test]
    fn shell_quote_handles_spaces_and_quotes() {
        assert_eq!(shell_quote("abc-123"), "abc-123");
        assert_eq!(shell_quote("two words"), "'two words'");
        assert_eq!(shell_quote("can't"), "'can'\\''t'");
    }
}
