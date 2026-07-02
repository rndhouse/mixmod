use crate::*;

pub fn init_project(root: &Path) -> Result<()> {
    ensure_project_state(root, true)
}

pub(crate) fn ensure_project_state(root: &Path, verbose: bool) -> Result<()> {
    let dirs = [
        ".mixmod/tasks",
        ".mixmod/runs",
        ".mixmod/experiments",
        MIXMOD_CODEX_HOME,
        ".mixmod/backups",
    ];

    if verbose {
        println!("Initializing Mixmod in {}", root.display());
    }
    for dir in dirs {
        let path = root.join(dir);
        if path.exists() {
            if verbose {
                println!("unchanged {}", dir);
            }
        } else {
            fs::create_dir_all(&path).with_context(|| format!("failed to create {dir}"))?;
            if verbose {
                println!("created {dir}");
            }
        }
    }

    write_managed_file(root, MIXMOD_CONFIG, &default_config_content(), verbose)?;
    write_managed_file(root, OPENCODE_CONFIG, &opencode_config_content(), verbose)?;
    Ok(())
}

pub fn status_project(root: &Path) -> Result<()> {
    println!("Mixmod status for {}", root.display());
    println!(
        "initialized: {}",
        yes_no(is_managed_file(&root.join(MIXMOD_CONFIG)))
    );
    println!("managed files:");
    for rel in [MIXMOD_CONFIG, OPENCODE_CONFIG] {
        let path = root.join(rel);
        let state = file_state(&path);
        println!("  {rel}: {state}");
    }
    println!(
        "codex home: {} ({})",
        yes_no(root.join(MIXMOD_CODEX_HOME).is_dir()),
        MIXMOD_CODEX_HOME
    );
    print_path_status("codex");
    print_path_status("opencode");
    Ok(())
}

pub fn doctor_project(root: &Path) -> Result<()> {
    let mut issues = Vec::new();
    println!("Mixmod doctor for {}", root.display());

    if is_managed_file(&root.join(MIXMOD_CONFIG)) {
        println!("ok: Mixmod config exists at {MIXMOD_CONFIG}");
    } else {
        println!("ok: Mixmod config will be created on first run at {MIXMOD_CONFIG}");
    }

    if is_managed_file(&root.join(OPENCODE_CONFIG)) {
        println!("ok: Mixmod OpenCode config exists at {OPENCODE_CONFIG}");
    } else {
        println!("ok: Mixmod OpenCode config will be created on first run at {OPENCODE_CONFIG}");
    }

    if find_on_path("codex").is_some() {
        println!("ok: codex found on PATH");
    } else {
        println!("error: codex was not found on PATH");
        println!("action: install Codex or add its binary directory to PATH");
        issues.push("codex missing");
    }

    let config = load_config(root).unwrap_or_default();
    let opencode_command = env::var("MIXMOD_OPENCODE_COMMAND")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(config.opencode.command);
    if find_on_path(&opencode_command).is_some() || Path::new(&opencode_command).exists() {
        println!("ok: OpenCode command `{opencode_command}` is available");
    } else {
        println!("error: OpenCode command `{opencode_command}` was not found");
        println!(
            "action: install OpenCode, add it to PATH, or set MIXMOD_OPENCODE_COMMAND / .mixmod/config.toml"
        );
        issues.push("opencode missing");
    }

    if issues.is_empty() {
        println!("doctor: ok");
        Ok(())
    } else {
        bail!(
            "doctor found {} issue(s): {}",
            issues.len(),
            issues.join(", ")
        )
    }
}

fn default_config_content() -> String {
    format!(
        r#"# BEGIN MIXMOD MANAGED: config
# Project-local Mixmod configuration. This file is intentionally repo-local.

[opencode]
provider = "{opencode_provider}"
model = "{default_model}"
require_local = true

# Override with MIXMOD_OPENCODE_COMMAND when needed.
command = "opencode"

# Placeholders:
# - {{instruction}}: full generated instruction text
# - {{instruction_file}}: path to the generated instruction file
# - {{task_file}}: path to the task JSON
# - {{mode}}: Mixmod delegation mode
# - {{out_dir}}: run artifact directory
# - {{model}}: resolved model id
# - {{provider}}: resolved provider id
# - {{model_arg}}: explicit provider/model argument passed to OpenCode
# - {{session_id}}: Mixmod-generated OpenCode session label
# - {{resume_session_id}}: real OpenCode session id used for worker_mode=continue
args = ["run", "--dangerously-skip-permissions", "--model", "{{model_arg}}", "--title", "{{session_id}}", "{{instruction}}"]

heartbeat_seconds = 10
worker_timeout_seconds = 600
idle_timeout_seconds = 300

local_providers = ["local", "{opencode_provider}", "local-ollama", "ollama", "lmstudio", "llama.cpp", "vllm", "localhost"]

[opencode.local_verification]
enabled = true
gpu_command = "nvidia-smi"
backend_command = "ollama ps"

[opencode.model_aliases]
"{default_model}" = ["{default_model}", "{ollama_model}", "qwen/qwen3.6-27b", "ollama/{ollama_model}", "local-ollama/{ollama_model}", "{opencode_provider}/{ollama_model}"]

[frontier]
model = "{frontier_model}"
# Codex config key: model_reasoning_effort. Allowed: minimal, low, medium, high, xhigh.
reasoning_effort = "{frontier_reasoning_effort}"
# END MIXMOD MANAGED: config
"#,
        opencode_provider = DEFAULT_OPENCODE_PROVIDER,
        default_model = DEFAULT_OPENCODE_MODEL,
        ollama_model = DEFAULT_OPENCODE_OLLAMA_MODEL,
        frontier_model = DEFAULT_FRONTIER_MODEL,
        frontier_reasoning_effort = DEFAULT_FRONTIER_REASONING_EFFORT
    )
}

fn opencode_config_content() -> String {
    opencode_config_content_for_provider(DEFAULT_OPENCODE_PROVIDER, "Ollama (Mixmod local)")
}

fn legacy_opencode_config_content() -> String {
    opencode_config_content_for_provider("local-ollama", "Ollama (repo-local)")
}

fn opencode_config_content_for_provider(provider: &str, name: &str) -> String {
    format!(
        r#"{{
  "$schema": "https://opencode.ai/config.json",
  "autoupdate": false,
  "model": "{provider}/{ollama_model}",
  "provider": {{
    "{provider}": {{
      "name": "{name}",
      "npm": "@ai-sdk/openai-compatible",
      "options": {{
        "baseURL": "http://127.0.0.1:11434/v1"
      }},
      "models": {{
        "{ollama_model}": {{
          "name": "Qwen 3.6 27B (local)"
        }}
      }}
    }}
  }}
}}
"#,
        provider = provider,
        name = name,
        ollama_model = DEFAULT_OPENCODE_OLLAMA_MODEL
    )
}

fn write_managed_file(root: &Path, rel: &str, content: &str, verbose: bool) -> Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    let existed = path.exists();
    if existed && !is_managed_file(&path) {
        if verbose {
            println!("left unmanaged {rel}");
        }
        return Ok(());
    }

    let current = fs::read_to_string(&path).unwrap_or_default();
    if current == content {
        if verbose {
            println!("unchanged {rel}");
        }
    } else {
        atomic_write(&path, content.as_bytes())?;
        if verbose {
            println!("{} {rel}", if existed { "updated" } else { "created" });
        }
    }
    Ok(())
}

pub(crate) fn load_config(root: &Path) -> Result<MixmodConfig> {
    let path = root.join(MIXMOD_CONFIG);
    if !path.exists() {
        return Ok(MixmodConfig::default());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

pub(crate) fn is_managed_file(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    if content.contains(MANAGED_MARKER) {
        return true;
    }
    let file_name = path.file_name().and_then(OsStr::to_str);
    file_name == Some("opencode.json")
        && (content.trim_end() == opencode_config_content().trim_end()
            || content.trim_end() == legacy_opencode_config_content().trim_end())
}

fn file_state(path: &Path) -> String {
    if !path.exists() {
        return "missing".to_string();
    }
    let managed = if is_managed_file(path) {
        "managed"
    } else {
        "unmanaged"
    };
    if is_executable(path) {
        format!("present, {managed}, executable")
    } else {
        format!("present, {managed}")
    }
}

fn print_path_status(bin: &str) {
    match find_on_path(bin) {
        Some(path) => println!("{bin} on PATH: yes ({})", path.display()),
        None => println!("{bin} on PATH: no"),
    }
}

pub(crate) fn find_on_path(bin: &str) -> Option<PathBuf> {
    let candidate = Path::new(bin);
    if candidate.components().count() > 1 {
        return candidate.exists().then(|| candidate.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(bin);
        if candidate.exists() && is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

pub(crate) fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
