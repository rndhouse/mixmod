use crate::*;

pub(super) fn experiment_dir(root: &Path, name: &str) -> PathBuf {
    state_layout(root).experiments().join(name)
}

pub(super) fn copy_budgeted_artifacts(
    root: &Path,
    budgeted_dir: &Path,
    final_out: &Path,
) -> Result<()> {
    for &name in WORKER_RUN_ARTIFACTS {
        let source = final_out.join(name);
        if source.exists() {
            fs::copy(&source, budgeted_dir.join(name)).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    display_path(root, &source),
                    display_path(root, &budgeted_dir.join(name))
                )
            })?;
        }
    }
    let logs_dir = budgeted_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create artifact logs dir {}", logs_dir.display()))?;
    for name in [
        OPENCODE_EVENTS_JSONL,
        "opencode.stdout.txt",
        "opencode.stderr.txt",
        "nvidia-smi-before.txt",
        "nvidia-smi-during.txt",
        "nvidia-smi-after.txt",
        "backend-status.txt",
        "heartbeat.jsonl",
    ] {
        let source = final_out.join("logs").join(name);
        if source.exists() {
            let target = logs_dir.join(name);
            fs::copy(&source, &target).with_context(|| {
                format!(
                    "failed to copy worker log {} to {}",
                    source.display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

pub(super) fn experiment_readme(name: &str) -> String {
    format!(
        r#"# Mixmod Experiment: {name}

This directory compares one small code-change task in two modes:

1. Mixmod default: the supervisor emits a compact executable worker handoff, the configured worker implements from the original task plus that handoff, and the supervisor reviews compact artifacts.
2. External baselines, when needed, should be run outside the Mixmod CLI and recorded separately.

Suggested workflow:

```sh
mixmod experiment run-default {name} --require-local
mixmod experiment report {name}
```

Mixmod default metrics are written under `default/`.
"#
    )
}

pub(super) fn task_md_template(name: &str) -> String {
    format!(
        r#"# {name}

## Task

Describe one bounded code-change task.

## Relevant files

- TBD

## Acceptance

- TBD

## Tests

- TBD
"#
    )
}

pub(super) fn task_json_template(name: &str) -> Value {
    json!({
        "title": name,
        "instructions": "Describe one bounded code-change task.",
        "files": [],
        "tests": [],
        "constraints": [
            "Keep the patch focused.",
            "Report tests clearly.",
            "Do not paste long logs."
        ],
        "acceptance": []
    })
}

pub(super) fn mixmod_notes_template(name: &str) -> String {
    format!(
        r#"# Mixmod Default Notes: {name}

Record:
- Codex turns:
- Mixmod delegations:
- Artifacts read by Codex:
- Whether Codex read `session.jsonl` or raw logs:
- Codex token usage, if available:
- Tests run:
- Final status:
- Notes:
"#
    )
}

pub(crate) fn placeholder_experiment_metrics(kind: &str) -> Value {
    json!({
        "kind": kind,
        "recorded_at": null,
        "codex_token_usage": null,
        "codex_turns": null,
        "mixmod_delegations": if kind == "codex-plus-mixmod" { 1 } else { 0 },
        "artifact_files_read_by_codex": [],
        "did_codex_read_full_mixmod_session": false,
        "approximate_codex_input_bytes": null,
        "approximate_codex_output_bytes": null,
        "local_worker_text_bytes": null,
        "patch_bytes": 0,
        "changed_file_count": 0,
        "changed_line_count": 0,
        "final_status": "unknown",
        "notes": ["Telemetry unavailable until this slot is recorded."]
    })
}

pub(super) fn copy_fixture_workdir(
    fixture: &Path,
    target: &Path,
    label: &str,
    root: &Path,
) -> Result<()> {
    if !fixture.is_dir() {
        bail!("fixture path is not a directory: {}", fixture.display());
    }
    if target.exists()
        && fs::read_dir(target)
            .with_context(|| format!("failed to inspect work dir {}", target.display()))?
            .next()
            .is_some()
    {
        println!(
            "unchanged {} work dir {}",
            label,
            display_path(root, target)
        );
        return Ok(());
    }
    fs::create_dir_all(target)
        .with_context(|| format!("failed to create {label} work dir {}", target.display()))?;
    copy_dir_contents(fixture, target, true)?;
    init_fixture_git_repo(target)?;
    println!("created {} work dir {}", label, display_path(root, target));
    Ok(())
}

pub(super) fn seed_experiment_task_from_fixture(
    fixture: &Path,
    exp_dir: &Path,
    root: &Path,
) -> Result<()> {
    let fixture_task = fixture.join(TASK_JSON);
    if !fixture_task.exists() {
        return Ok(());
    }
    let value = read_json_file(&fixture_task)?;
    write_pretty_json(&exp_dir.join(TASK_JSON), &value, "fixture task")?;
    atomic_write(
        &exp_dir.join(TASK_MD),
        task_markdown_from_json(&value).as_bytes(),
    )?;
    println!(
        "seeded experiment task from {}",
        display_path(root, &fixture_task)
    );
    Ok(())
}

fn copy_dir_contents(source: &Path, target: &Path, is_fixture_root: bool) -> Result<()> {
    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to read directory {}", source.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", source.display()))?;
        let source_path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == ".mixmod" || name == ".codex" {
            continue;
        }
        if is_fixture_root && (name == TASK_JSON || name == TASK_MD) {
            continue;
        }
        let target_path = target.join(name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::create_dir_all(&target_path)
                .with_context(|| format!("failed to create directory {}", target_path.display()))?;
            copy_dir_contents(&source_path, &target_path, false)?;
        } else if file_type.is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn init_fixture_git_repo(target: &Path) -> Result<()> {
    run_git(target, &["init"])?;
    run_git(target, &["config", "user.email", "mixmod@example.invalid"])?;
    run_git(target, &["config", "user.name", "Mixmod Fixture"])?;
    run_git(target, &["add", "."])?;
    run_git(target, &["commit", "-m", "fixture baseline"])?;
    Ok(())
}

fn run_git(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run git {} in {}", args.join(" "), root.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

pub(crate) fn validate_experiment_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        bail!(
            "invalid experiment name `{name}`; use ASCII letters, numbers, dot, underscore, or hyphen"
        );
    }
    Ok(())
}
