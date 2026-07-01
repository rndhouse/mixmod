use crate::*;

pub fn init_project(root: &Path) -> Result<()> {
    let mut manifest = load_backup_manifest(root)?;
    let dirs = [
        ".mixmod/tasks",
        ".mixmod/runs",
        ".mixmod/experiments",
        ".mixmod/codex-home",
        ".mixmod/backups",
        ".codex/hooks",
    ];

    println!("Initializing Mixmod in {}", root.display());
    for dir in dirs {
        let path = root.join(dir);
        if path.exists() {
            println!("unchanged {}", dir);
        } else {
            fs::create_dir_all(&path).with_context(|| format!("failed to create {dir}"))?;
            println!("created {dir}");
        }
    }

    write_managed_file(
        root,
        &mut manifest,
        MIXMOD_CONFIG,
        &default_config_content(),
        false,
    )?;
    write_managed_file(
        root,
        &mut manifest,
        OPENCODE_CONFIG,
        &opencode_config_content(),
        false,
    )?;
    write_managed_file(
        root,
        &mut manifest,
        CODEX_INSTRUCTIONS,
        &codex_instructions_content(),
        false,
    )?;
    write_managed_file(
        root,
        &mut manifest,
        CODEX_CONFIG,
        &codex_config_content(),
        false,
    )?;
    write_managed_file(
        root,
        &mut manifest,
        CODEX_HOOKS_CONFIG,
        &codex_hooks_content(),
        false,
    )?;
    write_managed_file(
        root,
        &mut manifest,
        CODEX_HOOK,
        &hook_script_content(),
        true,
    )?;
    save_backup_manifest(root, &manifest)?;
    Ok(())
}

pub fn uninstall_project(root: &Path) -> Result<()> {
    let mut manifest = load_backup_manifest(root)?;
    println!(
        "Uninstalling Mixmod repo-local integration from {}",
        root.display()
    );

    for rel in [
        CODEX_HOOK,
        CODEX_HOOKS_CONFIG,
        CODEX_CONFIG,
        CODEX_INSTRUCTIONS,
        OPENCODE_CONFIG,
        MIXMOD_CONFIG,
    ] {
        uninstall_managed_file(root, &mut manifest, rel)?;
    }

    remove_dir_if_empty(root.join(".codex/hooks"))?;
    remove_dir_if_empty(root.join(".codex"))?;
    remove_dir_if_empty(root.join(".mixmod/codex-home"))?;
    save_backup_manifest(root, &manifest)?;
    Ok(())
}

pub fn status_project(root: &Path) -> Result<()> {
    println!("Mixmod status for {}", root.display());
    println!(
        "initialized: {}",
        yes_no(is_managed_file(&root.join(MIXMOD_CONFIG)))
    );
    println!("managed files:");
    for rel in [
        MIXMOD_CONFIG,
        OPENCODE_CONFIG,
        CODEX_INSTRUCTIONS,
        CODEX_CONFIG,
        CODEX_HOOKS_CONFIG,
        CODEX_HOOK,
    ] {
        let path = root.join(rel);
        let state = file_state(&path);
        println!("  {rel}: {state}");
    }
    println!(
        "hooks installed: {}",
        yes_no(
            is_managed_file(&root.join(CODEX_HOOKS_CONFIG))
                && is_managed_file(&root.join(CODEX_HOOK))
                && is_executable(&root.join(CODEX_HOOK))
        )
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
        println!("warn: Mixmod is not initialized; run `mixmod init`");
        issues.push("Mixmod is not initialized");
    }

    if is_managed_file(&root.join(OPENCODE_CONFIG)) {
        println!("ok: repo-local OpenCode config exists at {OPENCODE_CONFIG}");
    } else {
        println!(
            "warn: repo-local OpenCode config is missing; run `mixmod init` to expose the default local model to OpenCode"
        );
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

    if env::var("MIXMOD_HOOK_DEPTH")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0)
        > 1
    {
        println!("warn: MIXMOD_HOOK_DEPTH is greater than 1; hook recursion guard is active");
        issues.push("hook recursion guard active");
    } else {
        println!("ok: hook recursion guard is not active");
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

pub fn hook_entrypoint(root: &Path, args: Vec<String>) -> Result<()> {
    if env::var("MIXMOD_DISABLE_HOOKS").as_deref() == Ok("1") {
        println!("mixmod hook: disabled by MIXMOD_DISABLE_HOOKS=1");
        return Ok(());
    }

    let depth = env::var("MIXMOD_HOOK_DEPTH")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    if depth > 1 {
        println!("mixmod hook: recursion guard active; passing through");
        return Ok(());
    }

    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    let log_path = root.join(".mixmod/hooks.jsonl");
    let event = json!({
        "timestamp": Utc::now().to_rfc3339(),
        "args": args,
        "stdin_bytes": stdin.len() as u64,
        "stdin_preview": truncate_for_report(&stdin, 1200),
        "depth": depth,
        "status": "pass-through"
    });
    append_jsonl(&log_path, &event)?;
    println!(
        "mixmod hook: logged pass-through event to {}",
        display_path(root, &log_path)
    );
    Ok(())
}

fn default_config_content() -> String {
    format!(
        r#"# BEGIN MIXMOD MANAGED: config
# Project-local Mixmod configuration. This file is intentionally repo-local.

[opencode]
provider = "local"
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

local_providers = ["local", "local-ollama", "ollama", "lmstudio", "llama.cpp", "vllm", "localhost"]

[opencode.local_verification]
enabled = true
gpu_command = "nvidia-smi"
backend_command = "ollama ps"

[opencode.model_aliases]
"{default_model}" = ["{default_model}", "{ollama_model}", "qwen/qwen3.6-27b", "ollama/{ollama_model}", "local-ollama/{ollama_model}"]

[frontier]
model = "{frontier_model}"
# Codex config key: model_reasoning_effort. Allowed: minimal, low, medium, high, xhigh.
reasoning_effort = "{frontier_reasoning_effort}"
# END MIXMOD MANAGED: config
"#,
        default_model = DEFAULT_OPENCODE_MODEL,
        ollama_model = DEFAULT_OPENCODE_OLLAMA_MODEL,
        frontier_model = DEFAULT_FRONTIER_MODEL,
        frontier_reasoning_effort = DEFAULT_FRONTIER_REASONING_EFFORT
    )
}

fn opencode_config_content() -> String {
    format!(
        r#"{{
  "$schema": "https://opencode.ai/config.json",
  "autoupdate": false,
  "model": "local-ollama/{ollama_model}",
  "provider": {{
    "local-ollama": {{
      "name": "Ollama (repo-local)",
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
        ollama_model = DEFAULT_OPENCODE_OLLAMA_MODEL
    )
}

fn codex_instructions_content() -> String {
    format!(
        "<!-- BEGIN MIXMOD MANAGED: instructions -->\n{INSTRUCTION_TEXT}<!-- END MIXMOD MANAGED: instructions -->\n"
    )
}

fn codex_config_content() -> String {
    r#"# BEGIN MIXMOD MANAGED: codex-config
# Conservative repo-local Mixmod integration metadata.
# Hooks are defined in .codex/hooks.json and route to the local Mixmod endpoint.

[mixmod]
enabled = true
instructions = ".codex/mixmod-instructions.md"
hooks = ".codex/hooks.json"
hook = ".codex/hooks/mixmod-hook.sh"
state = ".mixmod"
codex_home = ".mixmod/codex-home"
# END MIXMOD MANAGED: codex-config
"#
    .to_string()
}

fn codex_hooks_content() -> String {
    r#"{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup|resume",
        "hooks": [
          {
            "type": "command",
            "command": "\"$(git rev-parse --show-toplevel)/.codex/hooks/mixmod-hook.sh\" session-start",
            "timeout": 10,
            "statusMessage": "Mixmod session hook"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"$(git rev-parse --show-toplevel)/.codex/hooks/mixmod-hook.sh\" user-prompt-submit",
            "timeout": 10,
            "statusMessage": "Mixmod prompt hook"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"$(git rev-parse --show-toplevel)/.codex/hooks/mixmod-hook.sh\" stop",
            "timeout": 10,
            "statusMessage": "Mixmod stop hook"
          }
        ]
      }
    ]
  }
}
"#
    .to_string()
}

fn hook_script_content() -> String {
    r#"#!/usr/bin/env sh
# BEGIN MIXMOD MANAGED: hook
set -eu

depth="${MIXMOD_HOOK_DEPTH:-0}"
if [ "$depth" -gt 0 ]; then
  exit 0
fi

export MIXMOD_HOOK_DEPTH=1
exec mixmod hook "$@"
# END MIXMOD MANAGED: hook
"#
    .to_string()
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct BackupManifest {
    entries: Vec<BackupEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BackupEntry {
    path: String,
    backup: String,
    created_at: String,
}

fn write_managed_file(
    root: &Path,
    manifest: &mut BackupManifest,
    rel: &str,
    content: &str,
    executable: bool,
) -> Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    let existed = path.exists();
    if existed && !is_managed_file(&path) {
        let backup = backup_existing_file(root, manifest, rel)?;
        println!("backed up {rel} to {backup}");
    }

    let current = fs::read_to_string(&path).unwrap_or_default();
    if current == content {
        println!("unchanged {rel}");
    } else {
        atomic_write(&path, content.as_bytes())?;
        println!("{} {rel}", if existed { "updated" } else { "created" });
    }
    if executable {
        set_executable(&path)?;
    }
    Ok(())
}

fn uninstall_managed_file(root: &Path, manifest: &mut BackupManifest, rel: &str) -> Result<()> {
    let path = root.join(rel);
    if path.exists() {
        if is_managed_file(&path) {
            fs::remove_file(&path).with_context(|| format!("failed to remove {rel}"))?;
            println!("removed {rel}");
        } else {
            println!("left unmanaged {rel}");
        }
    } else {
        println!("absent {rel}");
    }

    if let Some(index) = manifest.entries.iter().position(|entry| entry.path == rel) {
        let entry = manifest.entries.remove(index);
        let backup_path = root.join(&entry.backup);
        if backup_path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            fs::copy(&backup_path, &path).with_context(|| {
                format!(
                    "failed to restore backup {} to {rel}",
                    display_path(root, &backup_path)
                )
            })?;
            println!("restored {rel} from {}", entry.backup);
        }
    }
    Ok(())
}

fn backup_existing_file(root: &Path, manifest: &mut BackupManifest, rel: &str) -> Result<String> {
    let backup_dir = root.join(".mixmod/backups");
    fs::create_dir_all(&backup_dir)
        .with_context(|| format!("failed to create backup directory {}", backup_dir.display()))?;
    manifest.entries.retain(|entry| entry.path != rel);
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let backup_rel = format!(".mixmod/backups/{}-{stamp}.bak", sanitize_path(rel));
    fs::copy(root.join(rel), root.join(&backup_rel))
        .with_context(|| format!("failed to back up {rel} to {backup_rel}"))?;
    manifest.entries.push(BackupEntry {
        path: rel.to_string(),
        backup: backup_rel.clone(),
        created_at: Utc::now().to_rfc3339(),
    });
    Ok(backup_rel)
}

fn load_backup_manifest(root: &Path) -> Result<BackupManifest> {
    let path = root.join(BACKUP_MANIFEST);
    if !path.exists() {
        return Ok(BackupManifest::default());
    }
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse backup manifest {}", path.display()))
}

fn save_backup_manifest(root: &Path, manifest: &BackupManifest) -> Result<()> {
    let path = root.join(BACKUP_MANIFEST);
    write_pretty_json(&path, manifest, "backup manifest")
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
    (file_name == Some(OPENCODE_CONFIG)
        && content.trim_end() == opencode_config_content().trim_end())
        || (path
            .to_string_lossy()
            .ends_with(CODEX_HOOKS_CONFIG.trim_start_matches("./"))
            && content.trim_end() == codex_hooks_content().trim_end())
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

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path.display()))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to set executable permissions on {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

pub(crate) fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
