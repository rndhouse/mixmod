use crate::*;
use serde_json::{Map, json};

pub fn init_project(root: &Path) -> Result<()> {
    ensure_project_state(root, true)
}

pub(crate) fn ensure_project_state(root: &Path, verbose: bool) -> Result<()> {
    let layout = state_layout(root);
    let dirs = [
        layout.tasks(),
        layout.runs(),
        layout.experiments(),
        layout.codex_home(),
        layout.backups(),
    ];

    if verbose {
        println!("Initializing Mixmod state for {}", root.display());
        println!("state: {}", layout.project_dir().display());
    }
    for dir in dirs {
        if dir.exists() {
            if verbose {
                println!("unchanged {}", dir.display());
            }
        } else {
            fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
            if verbose {
                println!("created {}", dir.display());
            }
        }
    }

    write_managed_file(
        &layout.config(),
        "config.toml",
        &default_config_content(),
        verbose,
    )?;
    write_managed_file(
        &layout.opencode_config(),
        "opencode.json",
        &opencode_config_content(),
        verbose,
    )?;
    Ok(())
}

pub fn status_project(root: &Path) -> Result<()> {
    let layout = state_layout(root);
    println!("Mixmod status for {}", root.display());
    println!("state root: {}", layout.state_root().display());
    println!("project state: {}", layout.project_dir().display());
    println!("initialized: {}", yes_no(is_managed_file(&layout.config())));
    println!("managed files:");
    println!("  config.toml: {}", file_state(&layout.config()));
    println!("  opencode.json: {}", file_state(&layout.opencode_config()));
    println!(
        "codex home: {} ({})",
        yes_no(layout.codex_home().is_dir()),
        layout.codex_home().display()
    );
    print_path_status("codex");
    print_path_status("opencode");
    Ok(())
}

pub fn doctor_project(root: &Path) -> Result<()> {
    let mut issues = Vec::new();
    let layout = state_layout(root);
    println!("Mixmod doctor for {}", root.display());
    println!("state: {}", layout.project_dir().display());

    if is_managed_file(&layout.config()) {
        println!("ok: Mixmod config exists at {}", layout.config().display());
    } else {
        println!(
            "ok: Mixmod config will be created on first run at {}",
            layout.config().display()
        );
    }

    if is_managed_file(&layout.opencode_config()) {
        println!(
            "ok: Mixmod OpenCode config exists at {}",
            layout.opencode_config().display()
        );
    } else {
        println!(
            "ok: Mixmod OpenCode config will be created on first run at {}",
            layout.opencode_config().display()
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
    let worker_backend = config.worker.backend;
    let opencode_command = env::var("MIXMOD_OPENCODE_COMMAND")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(config.opencode.command);
    if worker_backend == WorkerBackend::OpenCode {
        if find_on_path(&opencode_command).is_some() || Path::new(&opencode_command).exists() {
            println!("ok: OpenCode command `{opencode_command}` is available");
        } else {
            println!("error: OpenCode command `{opencode_command}` was not found");
            println!(
                "action: install OpenCode, add it to PATH, or set MIXMOD_OPENCODE_COMMAND / {}",
                layout.config().display()
            );
            issues.push("opencode missing");
        }
    } else {
        println!("ok: OpenCode command check skipped for worker.backend=codex");
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
# Mixmod configuration for one repository. This file is stored outside the repository.

[strategy]
supervisor_init = "compact"
worker_self_review = false

[strategy.live_supervision]
enabled = true
min_elapsed_seconds = 120
check_interval_seconds = 120
stale_after_seconds = 90
max_checks_per_worker = 3

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
args = ["run", "--agent", "{mixmod_agent}", "--dangerously-skip-permissions", "--format", "json", "--thinking", "--model", "{{model_arg}}", "--title", "{{session_id}}", "{{instruction}}"]

heartbeat_seconds = 10
worker_timeout_seconds = 600
idle_timeout_seconds = 300

local_providers = ["local", "{opencode_provider}", "lmstudio", "llama.cpp", "vllm", "localhost"]

[opencode.local_verification]
enabled = true
gpu_command = "nvidia-smi"
backend_command = "curl -fsS http://127.0.0.1:8080/v1/models"

[opencode.model_aliases]
"{default_model}" = ["{default_model}", "{local_model}", "{opencode_provider}/{local_model}"]

[[worker_model_profiles]]
model = "{default_model}"
aliases = ["{default_model}", "{local_model}", "{opencode_provider}/{local_model}"]
target_patch_lines = 100
max_patch_lines = 250
supervisor_guidance = [
  "This worker is cheap and useful for focused source edits, narrow repo inspection, compact command checks, and proposal turns; avoid broad autonomous design work.",
  "It can spend a while reasoning before editing; do not assume it is stalled while OpenCode is still producing reasoning, tool, or stdout activity.",
  "It can struggle with large effective context before explicit overflow; avoid asking it to reread many files, and use worker_mode=context_focus after context overflow, stale context, or repeated no-delta turns.",
  "Treat the files list as a likely read queue for this worker: it often opens every listed path before editing, so do not list large or generated files unless full-file reading is intended and context-safe.",
  "For broad expected-patch tasks, prefer worker_turn_shape=patch_request: choose the largest coherent source behavior it is likely to complete cleanly, usually one to three focused files, a bounded goal, deferred checks, and optional exact_edits or anchors/snippets only when precision saves supervisor output.",
  "When route or file choice is unclear, use worker_turn_shape=planning_probe with expect_patch=false: ask it to inspect one to three focused authored-source files or targeted command outputs and propose the next request; do not let it decide task completion.",
  "After multiple clean patch_request turns with useful deltas and no context pressure, broaden the next patch_request within the same shape; after messy, broad, or stalled turns, shrink the next slice.",
  "Prefer human-authored source edits. Keep generated artifacts, vendored files, lockfiles, snapshots, and build outputs out of the worker-owned patch until there is a deliberate generated-output step.",
  "For generated-code tasks, separate authored-source edits from generated-output updates: first request only the human-authored source patch, then use a later turn to run the generator or inspect generated output after a source diff exists.",
  "If generated output must be produced, ask for a command-run or regeneration step, not manual full-file inspection or manual editing of the generated file.",
  "It may produce directionally useful but messy parser, grammar, generated-code, or broad integration patches; inspect changed-file lists and patch stats before opening large diffs.",
  "It may miss end-to-end integration across slices. Before approval, check that helpers, options, parser/generated code, callers, state mutation, and error propagation are wired where the task requires.",
  "Do not trust compile success, non-empty diff, or the worker's summary as proof of task completion; require task-derived behavior evidence plus likely negative or edge-case coverage when behavior changed.",
  "For option or behavior families with a base path plus modifiers, start with the base behavior and add one modifier family later unless prior worker turns show it can safely combine more.",
  "For large functions or code-generation paths, describe the smallest local transformation; include a literal anchor only when it prevents worker wandering without much supervisor output.",
  "When tests cannot start because dependencies are missing, keep the worker focused on repo-level evidence and allowed commands instead of global environment repair.",
]

[[worker_model_profiles]]
model = "openrouter/z-ai/glm-5.2"
aliases = ["openrouter/z-ai/glm-5.2", "z-ai/glm-5.2"]
target_patch_lines = 300
max_patch_lines = 800
supervisor_guidance = [
  "This worker is capable, but may over-investigate when the handoff contains an apparent implementation constraint conflict or an unresolved toolchain choice.",
  "For generated-code, parser/compiler, toolchain, or similar trap-prone tasks, resolve the implementation route in the supervisor handoff before invoking the worker; do not ask the worker to discover whether the obvious route is viable.",
  "When the supervisor has selected a route, tell the worker to trust that route unless a direct compile, test, or command result proves it impossible.",
  "For broad expected-patch tasks, prefer worker_turn_shape=bounded_feature_slice with one concrete implementation path, one to three focused files, and the first reversible source edit named explicitly.",
  "Make the initial handoff patch-first: include the chosen strategy, the exact next behavior slice, the files to touch, and deferred checks; avoid leaving design forks for the worker to resolve before editing.",
  "If the worker starts toolchain archaeology, scratch-file probing, broad repo reading, or test-before-edit behavior without a diff, use live control to restate the chosen implementation route and request an immediate focused source edit.",
  "For revisions, anchor the next instruction to the current accumulated patch, preserve useful existing edits, and name the next missing behavior instead of restarting discovery.",
  "Before approval, check that the accumulated patch implements the requested end-to-end behavior, not just the first structural field or helper, and require focused behavior evidence for the main path plus likely invalid or edge case.",
]

[supervisor]
model = "{supervisor_model}"
# Codex config key: model_reasoning_effort. Allowed: minimal, low, medium, high, xhigh.
reasoning_effort = "{supervisor_reasoning_effort}"

[worker]
backend = "opencode"

[codex_worker]
model = "{supervisor_model}"
# Codex config key: model_reasoning_effort. Allowed: minimal, low, medium, high, xhigh.
reasoning_effort = "{supervisor_reasoning_effort}"
# END MIXMOD MANAGED: config
"#,
        opencode_provider = DEFAULT_OPENCODE_PROVIDER,
        mixmod_agent = MIXMOD_OPENCODE_AGENT,
        default_model = DEFAULT_OPENCODE_MODEL,
        local_model = DEFAULT_OPENCODE_LOCAL_MODEL,
        supervisor_model = DEFAULT_SUPERVISOR_MODEL,
        supervisor_reasoning_effort = DEFAULT_SUPERVISOR_REASONING_EFFORT
    )
}

fn opencode_config_content() -> String {
    opencode_config_content_for_provider(DEFAULT_OPENCODE_PROVIDER, "llama.cpp (Mixmod local)")
}

fn legacy_opencode_config_content() -> String {
    opencode_config_content_for_provider("local-ollama", "Ollama (repo-local)")
}

fn previous_legacy_opencode_config_content() -> String {
    previous_opencode_config_content_for_provider("local-ollama", "Ollama (repo-local)")
}

fn opencode_config_content_for_provider(provider: &str, name: &str) -> String {
    let mut agents = Map::new();
    agents.insert(
        MIXMOD_OPENCODE_AGENT.to_string(),
        json!({
            "description": "Mixmod supervised code worker",
            "mode": "primary",
            "prompt": mixmod_opencode_agent_prompt(),
            "permission": {
                "read": "allow",
                "glob": "allow",
                "grep": "allow",
                "list": "allow",
                "edit": "allow",
                "bash": "allow",
                "lsp": "allow",
                "task": "deny",
                "todowrite": "deny",
                "webfetch": "deny",
                "websearch": "deny",
                "skill": "deny",
                "question": "deny",
                "external_directory": "deny"
            }
        }),
    );
    let mut models = Map::new();
    models.insert(
        DEFAULT_OPENCODE_LOCAL_MODEL.to_string(),
        json!({
            "name": "Qwen 3.6 27B (llama.cpp)",
            "reasoning": true
        }),
    );
    let mut providers = Map::new();
    providers.insert(
        provider.to_string(),
        json!({
            "name": name,
            "npm": "@ai-sdk/openai-compatible",
            "options": {
                "baseURL": "http://127.0.0.1:8080/v1"
            },
            "models": models
        }),
    );
    let config = json!({
        "$schema": "https://opencode.ai/config.json",
        "autoupdate": false,
        "model": format!("{provider}/{local_model}", local_model = DEFAULT_OPENCODE_LOCAL_MODEL),
        "default_agent": MIXMOD_OPENCODE_AGENT,
        "agent": agents,
        "provider": providers
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&config).expect("generated OpenCode config should serialize")
    )
}

fn previous_opencode_config_content() -> String {
    previous_opencode_config_content_for_provider("mixmod-local-ollama", "Ollama (Mixmod local)")
}

fn previous_opencode_config_content_for_provider(provider: &str, name: &str) -> String {
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
        ollama_model = "qwen3.6:27b"
    )
}

fn mixmod_opencode_agent_prompt() -> &'static str {
    "You are the Mixmod worker. The supervisor model reviews your output and remains the final authority.\n\
Use the Mixmod worker task as the source of truth.\n\
When the task says `Expected repository patch: yes`, a plan, todo list, or explanation is not complete by itself. Read the relevant files, make the smallest necessary repository edits, and confirm the repository diff is non-empty before finalizing. If no patch is actually needed, say that explicitly and explain the blocker or reason compactly.\n\
When the task says `Expected repository patch: no`, do not invent edits; answer or investigate compactly as requested.\n\
Do not inspect Mixmod-managed state or artifact directories. Keep final output concise."
}

fn write_managed_file(path: &Path, label: &str, content: &str, verbose: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    let existed = path.exists();
    if existed && !is_managed_file(path) {
        if verbose {
            println!("left unmanaged {}", path.display());
        }
        return Ok(());
    }

    let current = fs::read_to_string(path).unwrap_or_default();
    if current == content {
        if verbose {
            println!("unchanged {label}");
        }
    } else {
        atomic_write(path, content.as_bytes())?;
        if verbose {
            println!("{} {label}", if existed { "updated" } else { "created" });
        }
    }
    Ok(())
}

pub(crate) fn load_config(root: &Path) -> Result<MixmodConfig> {
    let path = state_layout(root).config();
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
            || content.trim_end() == previous_opencode_config_content().trim_end()
            || content.trim_end() == legacy_opencode_config_content().trim_end()
            || content.trim_end() == previous_legacy_opencode_config_content().trim_end())
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
