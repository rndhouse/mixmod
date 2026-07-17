use super::*;

#[test]
fn init_manages_only_central_state_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".codex")).unwrap();
    fs::write(root.join(".codex/config.toml"), "existing = true\n").unwrap();
    fs::write(root.join(LEGACY_OPENCODE_CONFIG), "{\"user\":true}\n").unwrap();

    init_project(root).unwrap();
    let layout = state_layout(root);
    assert!(is_managed_file(&layout.config()));
    assert!(is_managed_file(&layout.opencode_config()));
    assert!(layout.codex_home().is_dir());
    assert!(layout.backups().is_dir());
    assert!(!root.join(".mixmod").exists());
    assert!(!root.join(CODEX_INSTRUCTIONS).exists());
    assert!(!root.join(".codex/hooks.json").exists());
    assert!(!root.join(".codex/hooks").exists());
    assert_eq!(
        fs::read_to_string(root.join(".codex/config.toml")).unwrap(),
        "existing = true\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(LEGACY_OPENCODE_CONFIG)).unwrap(),
        "{\"user\":true}\n"
    );
}

#[test]
fn init_config_does_not_include_worker_model_profile_blocks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    init_project(root).unwrap();

    let config = fs::read_to_string(state_layout(root).config()).unwrap();
    assert!(!config.contains("worker_model_profiles"));
    assert!(!config.contains("supervisor_guidance"));
}

#[test]
fn init_config_enables_spin_out_supervisor_review_by_default() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    init_project(root).unwrap();

    let config_text = fs::read_to_string(state_layout(root).config()).unwrap();
    let config = load_config(root).unwrap();
    assert!(config_text.contains("spin_out_supervisor_review = true"));
    assert!(config.strategy.spin_out_supervisor_review);
}

#[test]
fn codex_app_server_uses_mixmod_scoped_codex_home() {
    assert_eq!(
        codex_home_for_work_dir(Path::new("/tmp/work")),
        state_layout(Path::new("/tmp/work")).codex_home()
    );
}

#[test]
fn opencode_uses_mixmod_scoped_config() {
    assert_eq!(
        opencode_config_path(Path::new("/tmp/work")),
        state_layout(Path::new("/tmp/work")).opencode_config()
    );
}

#[test]
fn opencode_config_is_written_to_central_state() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    init_project(root).unwrap();
    let config_path = state_layout(root).opencode_config();
    let config = read_json_file(&config_path).unwrap();

    assert!(config_path.starts_with(state_layout(root).project_dir()));
    assert!(!root.join("opencode.json").exists());
    assert_eq!(get_bool(&config, "autoupdate"), Some(false));
    assert_eq!(get_bool(&config["compaction"], "prune"), Some(true));
    assert_eq!(
        get_str(&config, "default_agent"),
        Some(MIXMOD_OPENCODE_AGENT)
    );
    let agent = &config["agent"][MIXMOD_OPENCODE_AGENT];
    assert_eq!(get_str(agent, "mode"), Some("primary"));
    assert!(
        agent.get("model").is_none(),
        "worker model should stay controlled by --worker-model / --model"
    );
    let prompt = get_str(agent, "prompt").unwrap_or_default();
    assert!(prompt.contains("Expected repository patch: yes"));
    assert!(prompt.contains("Expected repository patch: no"));
    assert!(prompt.contains("full-file read tool is disabled"));
    assert!(prompt.contains("Do not print whole large files"));
    let permission = &agent["permission"];
    assert_eq!(get_str(permission, "read"), Some("deny"));
    assert_eq!(get_str(permission, "grep"), Some("allow"));
    assert_eq!(get_str(permission, "glob"), Some("allow"));
    assert_eq!(get_str(permission, "edit"), Some("allow"));
    assert_eq!(get_str(permission, "bash"), Some("allow"));
    assert_eq!(get_str(permission, "todowrite"), Some("deny"));
    assert_eq!(get_str(permission, "task"), Some("deny"));
}

#[test]
fn init_updates_previous_mixmod_opencode_read_allow_config() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let layout = state_layout(root);
    fs::create_dir_all(layout.project_dir()).unwrap();
    atomic_write(
        &layout.opencode_config(),
        br#"{
  "$schema": "https://opencode.ai/config.json",
  "agent": {
    "mixmod-worker": {
      "description": "Mixmod supervised code worker",
      "mode": "primary",
      "permission": {
        "bash": "allow",
        "edit": "allow",
        "external_directory": "deny",
        "glob": "allow",
        "grep": "allow",
        "list": "allow",
        "lsp": "allow",
        "question": "deny",
        "read": "allow",
        "skill": "deny",
        "task": "deny",
        "todowrite": "deny",
        "webfetch": "deny",
        "websearch": "deny"
      },
      "prompt": "You are the Mixmod worker. The supervisor model reviews your output and remains the final authority.\nUse the Mixmod worker task as the source of truth.\nWhen the task says `Expected repository patch: yes`, a plan, todo list, or explanation is not complete by itself. Read the relevant files, make the smallest necessary repository edits, and confirm the repository diff is non-empty before finalizing. If no patch is actually needed, say that explicitly and explain the blocker or reason compactly.\nWhen the task says `Expected repository patch: no`, do not invent edits; answer or investigate compactly as requested.\nDo not inspect Mixmod-managed state or artifact directories. Keep final output concise."
    }
  },
  "autoupdate": false,
  "default_agent": "mixmod-worker",
  "model": "mixmod-local-ollama/qwen3.6:27b",
  "provider": {
    "mixmod-local-ollama": {
      "models": {
        "qwen3.6:27b": {
          "name": "Qwen 3.6 27B (local)",
          "reasoning": true
        }
      },
      "name": "Ollama (Mixmod local)",
      "npm": "@ai-sdk/openai-compatible",
      "options": {
        "baseURL": "http://127.0.0.1:11434/v1"
      }
    }
  }
}
"#,
    )
    .unwrap();

    init_project(root).unwrap();

    let config = read_json_file(&layout.opencode_config()).unwrap();
    let agent = &config["agent"][MIXMOD_OPENCODE_AGENT];
    assert_eq!(get_str(&agent["permission"], "read"), Some("deny"));
    assert!(
        get_str(agent, "prompt")
            .unwrap_or_default()
            .contains("full-file read tool is disabled")
    );
    assert_eq!(
        get_str(&config, "model"),
        Some("llama.cpp/qwen/qwen3.6-27b")
    );
}

#[test]
fn init_does_not_write_repo_local_git_exclude() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git(root);
    let exclude = root.join(".git/info/exclude");
    fs::write(&exclude, "# local ignores\n").unwrap();

    init_project(root).unwrap();
    init_project(root).unwrap();

    let content = fs::read_to_string(exclude).unwrap();
    assert_eq!(content, "# local ignores\n");
}
