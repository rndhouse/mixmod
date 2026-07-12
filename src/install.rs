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
supervisor_guidance = [
  "This local Qwen worker is much less capable than the supervisor, but it is effectively zero marginal GPT-token cost; use it as a cheap tool for bounded work, not as the strategic owner.",
  "This worker can spend a while reasoning before editing; do not assume it is stalled while OpenCode is still producing reasoning, tool, or stdout activity.",
  "This worker can struggle with large effective context before an explicit overflow occurs; keep initial handoffs compact, split broad tasks into small concrete source slices, and avoid asking it to reread many files at once.",
  "Treat this worker as a narrow local tool operator. Prefer worker_role=inspect for exact file/symbol/line discovery, worker_role=run_checks for command execution and failure summaries, and worker_role=patch_slice only for a concrete bounded edit.",
  "Prefer concrete command-style local help over open-ended review prompts; Qwen may keep analyzing broad review asks until timeout even after it has produced useful evidence.",
  "Qwen is weak at open-ended final diff review: it may spend the call reading large diffs or rerunning visible tests instead of finding missing behavior. For final verification, give it concrete commands or probes chosen by the supervisor, or one tiny branch-specific question.",
  "For bounded review questions, avoid prompts that invite whole-file reads. Ask for bounded snippets instead: git diff for named files, rg/grep around anchors, or sed ranges around the changed branch.",
  "For parser, binding, destructuring, or assignment changes, do not approve from the main happy path alone; check alternate syntax/input shapes such as single target versus multi-target, scalar versus aggregate or multi-value RHS, and relevant scope writes.",
  "For those syntax or assignment changes, ordinary package tests are insufficient unless they exercise the relevant alternate shape or the supervisor has direct code evidence that the alternate shape follows the same path.",
  "For evaluator-style tasks, prefer temporary probes or uniquely named regression tests; avoid generic top-level helper names that could collide with hidden tests in the same package.",
  "Use no-diff roles before patching when repo facts would otherwise cost supervisor context: inspect should return exact files, symbols, anchors, and uncertainty; run_checks should return commands, exit status, and compact failure excerpts.",
  "Do not ask this worker to own architecture, broad diagnosis, or final correctness. The supervisor should choose the strategy and use Qwen to gather evidence or execute a narrow edit.",
  "When worker_session_token_peak is high for the configured context window, treat the current worker session as context-pressured; shrink the next revision or use worker_mode=context_focus if the next edit would require broad rereading.",
  "For broad expected-patch tasks, use worker_turn_shape=small_patch_slice by default with one immediate source edit, one focused source file, a literal nearby anchor when available, no tests before a diff exists, and a compact edit packet/snippet so the worker can patch before broad exploration.",
  "When giving a small_patch_slice, tell it to use the provided edit packet first and avoid reading whole large files before the first edit.",
  "For revision small_patch_slice turns, make the next instruction executable from the current accumulated patch: preserve useful existing edits, name the one next source delta, and avoid telling the worker to restart from an earlier completed slice.",
  "For large functions or code-generation paths, provide one literal anchor plus the smallest local transformation near that anchor; avoid asking for an entire behavior path when a preparatory branch or helper would create useful progress.",
  "For alias/key generated-code repairs, hand off one path at a time such as valid-key collection, serialization key mapping, deserialization key mapping, or collision detection; when the source API permits either form, tell the worker to preserve both raw field names and resolved aliases.",
  "For option families or behavior families with a base path plus modifiers, ask for the base behavior first and then one modifier family per later small_patch_slice unless prior worker turns show it can safely combine them.",
  "After multiple clean small_patch_slice revisions with non-empty accurate deltas, no context overflow, and moderate token peak, consider the previous slices too small; promote within small_patch_slice to one coherent anchored source behavior instead of switching this profile to bounded_feature_slice.",
  "If a small_patch_slice required live supervisor control, produced a large line delta, or needed a corrective follow-up, treat the prior slice as too broad; shrink the next revision and do not add another modifier family or validation concern until one clean corrective delta lands.",
  "Once API plumbing and basic validation exist, prioritize the first useful behavior path over additional defensive validation slices unless the artifacts show validation is blocking progress.",
  "For revisions, prefer worker_mode=continue only while the worker context remains useful. If artifacts show context overflow, repeated summary updates, or no new delta after a focused revision, prefer worker_mode=context_focus with a smaller concrete source slice.",
  "When tests fail to start because dependencies are missing, keep it focused on repo-level evidence and allowed commands instead of global environment repair.",
  "It can create broad or malformed tests when fixture semantics are unclear; ask for the narrowest regression test that matches existing test style.",
  "It may try to mutate user or global environments while installing dependencies; prefer existing project commands and avoid global installs unless the task explicitly requires them.",
  "Before accepting a turn, check whether the intended repo diff exists and touches the expected source/test files.",
]

[[worker_model_profiles]]
model = "openrouter/z-ai/glm-5.2"
aliases = ["openrouter/z-ai/glm-5.2", "z-ai/glm-5.2"]
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
