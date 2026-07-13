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
fn codex_app_server_uses_non_temp_scoped_codex_home() {
    let layout = state_layout(Path::new("/tmp/work"));
    let codex_home = codex_home_for_work_dir(Path::new("/tmp/work"));

    assert!(codex_home.ends_with(layout.project_dir().file_name().unwrap()));
    if layout.codex_home().starts_with(std::env::temp_dir()) {
        assert!(!codex_home.starts_with(std::env::temp_dir()));
    }
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
    let permission = &agent["permission"];
    assert_eq!(get_str(permission, "read"), Some("allow"));
    assert_eq!(get_str(permission, "grep"), Some("allow"));
    assert_eq!(get_str(permission, "glob"), Some("allow"));
    assert_eq!(get_str(permission, "edit"), Some("allow"));
    assert_eq!(get_str(permission, "bash"), Some("allow"));
    assert_eq!(get_str(permission, "todowrite"), Some("deny"));
    assert_eq!(get_str(permission, "task"), Some("deny"));
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
