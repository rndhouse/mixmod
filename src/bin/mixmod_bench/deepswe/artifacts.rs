use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// Normalize a Pier job from CLI arguments and print bundle records as JSON.
pub(crate) fn normalize_cli(pool: &Path, job_dir: &Path, records_json: Option<&str>) -> Result<()> {
    let records = read_records(records_json)?;
    let bundles = normalize_pier_job(pool, job_dir, &records)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&bundles).context("failed to serialize bundles")?
    );
    Ok(())
}

/// Copy Pier trial artifacts into a stable per-task review layout.
fn normalize_pier_job(pool: &Path, job_dir: &Path, records: &[Value]) -> Result<Vec<Value>> {
    if !job_dir.exists() {
        return Ok(Vec::new());
    }

    let copied_job_files = copy_job_files(pool, job_dir)?;
    let mut task_bundles = Vec::new();
    let mut used_names = BTreeSet::new();
    for trial_dir in trial_dirs(job_dir)? {
        let task_name = task_name_from_trial(&trial_dir);
        let bundle_name =
            unique_bundle_name(&task_name, &trial_dir_name(&trial_dir), &mut used_names);
        let target = pool.join("tasks").join(bundle_name);
        let related_records = records_for_trial(records, &trial_dir);
        task_bundles.push(copy_trial_bundle(
            &trial_dir,
            &target,
            &task_name,
            &related_records,
        )?);
    }

    let manifest = json!({
        "kind": "deepswe-pier-artifact-bundles",
        "job_name": file_name_string(pool),
        "job_dir": job_dir.to_string_lossy(),
        "tasks_dir": pool.join("tasks").to_string_lossy(),
        "pier_job_files": copied_job_files,
        "task_bundles": task_bundles,
    });
    write_json(&pool.join("artifact-bundles.json"), &manifest)?;
    update_latest_pointer(pool)?;
    Ok(task_bundles)
}

fn read_records(records_json: Option<&str>) -> Result<Vec<Value>> {
    let Some(records_json) = records_json else {
        return Ok(Vec::new());
    };
    let mut text = String::new();
    if records_json == "-" {
        io::stdin()
            .read_to_string(&mut text)
            .context("failed to read records from stdin")?;
    } else {
        File::open(records_json)
            .with_context(|| format!("failed to open {records_json}"))?
            .read_to_string(&mut text)
            .with_context(|| format!("failed to read {records_json}"))?;
    }
    let value: Value = serde_json::from_str(&text).context("failed to parse records JSON")?;
    Ok(value.as_array().cloned().unwrap_or_default())
}

fn copy_job_files(pool: &Path, job_dir: &Path) -> Result<Vec<String>> {
    let target = pool.join("pier-job");
    fs::create_dir_all(&target)
        .with_context(|| format!("failed to create {}", target.display()))?;
    let mut copied = Vec::new();
    for source in sorted_entries(job_dir)? {
        if source.is_file()
            && let Some(copied_path) = copy_path(&source, &target.join(file_name(&source)))?
        {
            copied.push(relative_to(pool, &copied_path));
        }
    }
    Ok(copied)
}

fn trial_dirs(job_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for path in sorted_entries(job_dir)? {
        if path.is_dir() && (path.join("agent").exists() || path.join("config.json").exists()) {
            dirs.push(path);
        }
    }
    Ok(dirs)
}

fn task_name_from_trial(trial_dir: &Path) -> String {
    let config = load_json(&trial_dir.join("config.json"));
    if let Some(task) = config.get("task").and_then(Value::as_object) {
        if let Some(task_path) = task
            .get("path")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            return Path::new(task_path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(task_path)
                .to_string();
        }
        if let Some(task_name) = task
            .get("name")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            return task_name
                .rsplit('/')
                .next()
                .unwrap_or(task_name)
                .to_string();
        }
    }

    let result = load_json(&trial_dir.join("result.json"));
    if let Some(task_id) = result.get("task_id").and_then(Value::as_object)
        && let Some(task_path) = task_id
            .get("path")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    {
        return Path::new(task_path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(task_path)
            .to_string();
    }
    if let Some(task_name) = result
        .get("task_name")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        return task_name
            .rsplit('/')
            .next()
            .unwrap_or(task_name)
            .to_string();
    }

    let trial_name = trial_dir_name(trial_dir);
    if let Some((prefix, _)) = trial_name.split_once("__") {
        return prefix.to_string();
    }
    trial_name
}

fn unique_bundle_name(
    task_name: &str,
    trial_name: &str,
    used_names: &mut BTreeSet<String>,
) -> String {
    let base = sanitize_name(task_name);
    if used_names.insert(base.clone()) {
        return base;
    }
    let fallback = sanitize_name(trial_name);
    if used_names.insert(fallback.clone()) {
        return fallback;
    }
    let mut index = 2;
    loop {
        let unique = format!("{fallback}-{index}");
        if used_names.insert(unique.clone()) {
            return unique;
        }
        index += 1;
    }
}

fn copy_trial_bundle(
    trial_dir: &Path,
    target: &Path,
    task_name: &str,
    related_records: &[Value],
) -> Result<Value> {
    if target.exists() || is_symlink(target) {
        remove_path(target)?;
    }
    fs::create_dir_all(target).with_context(|| format!("failed to create {}", target.display()))?;

    let mut copied = BTreeMap::new();
    for name in ["agent", "artifacts", "verifier"] {
        if let Some(copied_path) = copy_path(&trial_dir.join(name), &target.join(name))? {
            copied.insert(name.to_string(), relative_to(target, &copied_path));
        }
    }

    let pier_dir = target.join("pier");
    for source in sorted_entries(trial_dir)? {
        if source.is_file() {
            let destination = pier_dir.join(file_name(&source));
            if let Some(copied_path) = copy_path(&source, &destination)? {
                copied.insert(
                    format!("pier/{}", file_name_string(&source)),
                    relative_to(target, &copied_path),
                );
            }
        }
    }

    let worker_runs = worker_runs(&target.join("agent/worker-runs"))?;
    let tool_proxy_runs = tool_proxy_runs(&target.join("agent/tool-proxy-runs"))?;
    let manifest = json!({
        "kind": "deepswe-task-artifact-bundle",
        "task": task_name,
        "trial_name": trial_dir_name(trial_dir),
        "source_trial_dir": trial_dir.to_string_lossy(),
        "bundle_dir": target.to_string_lossy(),
        "copied": copied,
        "records": related_records,
        "important_artifacts": important_artifacts(target),
        "worker_runs": worker_runs,
        "tool_proxy_runs": tool_proxy_runs,
    });
    write_json(&target.join("artifact-manifest.json"), &manifest)?;
    Ok(json!({
        "task": task_name,
        "trial_name": trial_dir_name(trial_dir),
        "bundle_dir": target.to_string_lossy(),
        "manifest": target.join("artifact-manifest.json").to_string_lossy(),
        "records": related_records,
        "worker_run_count": manifest["worker_runs"].as_array().map(Vec::len).unwrap_or(0),
        "latest_worker_run": manifest["worker_runs"].as_array()
            .and_then(|runs| runs.last())
            .and_then(|run| run.get("name"))
            .cloned()
            .unwrap_or(Value::Null),
        "tool_proxy_run_count": manifest["tool_proxy_runs"].as_array().map(Vec::len).unwrap_or(0),
        "latest_tool_proxy_run": manifest["tool_proxy_runs"].as_array()
            .and_then(|runs| runs.last())
            .and_then(|run| run.get("path"))
            .cloned()
            .unwrap_or(Value::Null),
    }))
}

fn records_for_trial(records: &[Value], trial_dir: &Path) -> Vec<Value> {
    let Ok(trial_prefix) = trial_dir.canonicalize() else {
        return Vec::new();
    };
    records
        .iter()
        .filter(|record| {
            record
                .as_object()
                .map(|object| {
                    object.iter().any(|(key, value)| {
                        (key.ends_with("_json")
                            || matches!(key.as_str(), "test_stdout" | "reward_json"))
                            && path_is_under(value, &trial_prefix)
                    })
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

fn path_is_under(value: &Value, parent: &Path) -> bool {
    let Some(raw) = value.as_str().filter(|value| !value.is_empty()) else {
        return false;
    };
    let Ok(path) = Path::new(raw).canonicalize() else {
        return false;
    };
    path.strip_prefix(parent).is_ok()
}

fn important_artifacts(target: &Path) -> BTreeMap<String, String> {
    let names = [
        ("supervisor_feedback", "agent/supervisor-feedback.jsonl"),
        (
            "supervision_loop_summary",
            "agent/supervision-loop-summary.json",
        ),
        ("worker_brief", "agent/worker-brief.json"),
        ("worker_task", "agent/worker-task.json"),
        ("mixmod_summary", "agent/mixmod-summary.json"),
        ("mixmod_metrics", "agent/mixmod-metrics.json"),
        ("mixmod_final_patch", "agent/mixmod-final.patch"),
        ("tool_proxy_runs", "agent/tool-proxy-runs"),
        ("model_patch", "artifacts/model.patch"),
        ("git_status", "artifacts/git-status.txt"),
    ];
    names
        .into_iter()
        .filter_map(|(key, path)| {
            let path = target.join(path);
            path.exists()
                .then(|| (key.to_string(), relative_to(target, &path)))
        })
        .collect()
}

fn tool_proxy_runs(tool_proxy_root: &Path) -> Result<Vec<Value>> {
    if !tool_proxy_root.exists() {
        return Ok(Vec::new());
    }
    let run_root = tool_proxy_root
        .parent()
        .and_then(Path::parent)
        .unwrap_or(tool_proxy_root);
    let mut run_dirs = Vec::new();
    collect_tool_proxy_run_dirs(tool_proxy_root, &mut run_dirs)?;
    run_dirs.sort_by_key(|path| tool_proxy_run_sort_key(path, run_root));

    run_dirs
        .into_iter()
        .map(|run_dir| {
            let mut artifact_sizes = BTreeMap::new();
            for name in [
                "opencode-instructions.md",
                "reasoning-trace.jsonl",
                "tool-events.jsonl",
                "tool-task.json",
                "report.md",
                "metrics.json",
                "changes.patch",
                "worktree.patch",
                "receipt.json",
                "session.jsonl",
                "supervisor-control.jsonl",
                "logs/opencode.stdout.txt",
                "logs/opencode.stderr.txt",
                "logs/heartbeat.jsonl",
            ] {
                let path = run_dir.join(name);
                if path.exists() {
                    artifact_sizes.insert(name.to_string(), path_size(&path)?);
                }
            }
            let tool_output_dir = run_dir.join("tool-output");
            if tool_output_dir.exists() {
                artifact_sizes.insert("tool-output".to_string(), path_size(&tool_output_dir)?);
            }
            let task = load_json_if_exists(&run_dir.join("tool-task.json"));
            let metrics = load_json_if_exists(&run_dir.join("metrics.json"));
            Ok(json!({
                "name": file_name_string(&run_dir),
                "path": relative_to(run_root, &run_dir),
                "completed": metrics.is_some(),
                "status": metrics.as_ref()
                    .and_then(|metrics| metrics.get("status"))
                    .and_then(Value::as_str),
                "changed_file_count": metrics.as_ref()
                    .and_then(|metrics| metrics.get("changed_file_count"))
                    .and_then(Value::as_u64),
                "changed_line_count": metrics.as_ref()
                    .and_then(|metrics| metrics.get("changed_line_count"))
                    .and_then(Value::as_u64),
                "kind": tool_proxy_task_kind(task.as_ref()),
                "worker_role": task.as_ref()
                    .and_then(|task| task.pointer("/context/worker_role"))
                    .and_then(Value::as_str),
                "artifact_sizes": artifact_sizes,
            }))
        })
        .collect()
}

fn collect_tool_proxy_run_dirs(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if root.join("metrics.json").exists() || root.join("opencode-instructions.md").exists() {
        out.push(root.to_path_buf());
        return Ok(());
    }
    for entry in sorted_entries(root)? {
        if entry.is_dir() {
            collect_tool_proxy_run_dirs(&entry, out)?;
        }
    }
    Ok(())
}

fn worker_runs(worker_root: &Path) -> Result<Vec<Value>> {
    if !worker_root.exists() {
        return Ok(Vec::new());
    }
    let mut worker_dirs = sorted_entries(worker_root)?
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    worker_dirs.sort_by_key(|path| worker_dir_sort_key(path));

    worker_dirs
        .into_iter()
        .map(|worker_dir| {
            let mut artifact_sizes = BTreeMap::new();
            for name in [
                "opencode-instructions.md",
                "reasoning-trace.jsonl",
                "tool-events.jsonl",
                "report.md",
                "metrics.json",
                "changes.patch",
                "worktree.patch",
                "patch-comparison.json",
                "patch-rollback.json",
                "rollback-current.patch",
                "rollback-restored.patch",
                "supervisor-control.jsonl",
                "logs/opencode.stdout.txt",
                "logs/opencode.stderr.txt",
                "logs/heartbeat.jsonl",
            ] {
                let path = worker_dir.join(name);
                if path.exists() {
                    artifact_sizes.insert(name.to_string(), path_size(&path)?);
                }
            }
            let tool_output_dir = worker_dir.join("tool-output");
            if tool_output_dir.exists() {
                artifact_sizes.insert("tool-output".to_string(), path_size(&tool_output_dir)?);
            }
            let run_root = worker_root
                .parent()
                .and_then(Path::parent)
                .unwrap_or(worker_root);
            Ok(json!({
                "name": file_name_string(&worker_dir),
                "path": relative_to(run_root, &worker_dir),
                "artifact_sizes": artifact_sizes,
            }))
        })
        .collect()
}

fn worker_dir_sort_key(path: &Path) -> (u8, u32, String) {
    let name = file_name_string(path);
    if name == "proposal" {
        return (0, 0, name);
    }
    if name == "revision" {
        return (1, 1, name);
    }
    if let Some(suffix) = name
        .strip_prefix("revision-")
        .and_then(|value| value.parse::<u32>().ok())
    {
        return (1, suffix, name);
    }
    (2, 0, name)
}

fn tool_proxy_run_sort_key(path: &Path, run_root: &Path) -> (String, String) {
    let name = file_name_string(path);
    (
        tool_proxy_run_timestamp(&name).unwrap_or_else(|| name.clone()),
        relative_to(run_root, path),
    )
}

fn tool_proxy_run_timestamp(name: &str) -> Option<String> {
    name.split_once('-')
        .and_then(|(_, timestamp)| (!timestamp.is_empty()).then(|| timestamp.to_string()))
}

fn load_json_if_exists(path: &Path) -> Option<Value> {
    path.exists().then(|| load_json(path))
}

fn tool_proxy_task_kind(task: Option<&Value>) -> &'static str {
    if task
        .and_then(|task| task.pointer("/context/worker_prompt"))
        .and_then(Value::as_str)
        .is_some()
    {
        "ask"
    } else if task
        .and_then(|task| task.pointer("/context/original_command"))
        .and_then(Value::as_str)
        .is_some()
    {
        "command"
    } else {
        "unknown"
    }
}

fn copy_path(source: &Path, target: &Path) -> Result<Option<PathBuf>> {
    if !source.exists() && !is_symlink(source) {
        return Ok(None);
    }
    if target.exists() || is_symlink(target) {
        remove_path(target)?;
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("failed to inspect {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        let destination = fs::read_link(source)
            .with_context(|| format!("failed to read symlink {}", source.display()))?;
        symlink(&destination, target)
            .with_context(|| format!("failed to copy symlink {}", source.display()))?;
    } else if metadata.is_dir() {
        copy_dir(source, target)?;
    } else {
        fs::copy(source, target).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                target.display()
            )
        })?;
    }
    Ok(Some(target.to_path_buf()))
}

fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target).with_context(|| format!("failed to create {}", target.display()))?;
    for child in sorted_entries(source)? {
        copy_path(&child, &target.join(file_name(&child)))?;
    }
    Ok(())
}

fn path_size(path: &Path) -> Result<u64> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    let mut total = 0;
    for child in sorted_entries(path)? {
        total += path_size(&child)?;
    }
    Ok(total)
}

#[cfg(unix)]
fn symlink(source: &Path, target: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(not(unix))]
fn symlink(source: &Path, target: &Path) -> io::Result<()> {
    let _ = source;
    let _ = target;
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symlink copy is only supported on Unix",
    ))
}

fn remove_path(path: &Path) -> Result<()> {
    if is_symlink(path) || path.is_file() {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn update_latest_pointer(pool: &Path) -> Result<()> {
    let parent = pool.parent().unwrap_or_else(|| Path::new("."));
    let latest = parent.join("latest");
    let latest_txt = parent.join("latest.txt");
    fs::write(&latest_txt, format!("{}\n", file_name_string(pool)))
        .with_context(|| format!("failed to write {}", latest_txt.display()))?;
    if latest.exists() && !is_symlink(&latest) && latest.is_dir() {
        return Ok(());
    }
    if latest.exists() || is_symlink(&latest) {
        remove_path(&latest)?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(file_name(pool), &latest).ok();
    }
    Ok(())
}

fn load_json(path: &Path) -> Value {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut bytes = serde_json::to_vec_pretty(value).context("failed to serialize JSON")?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn sorted_entries(path: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()
        .with_context(|| format!("failed to read entry in {}", path.display()))?;
    entries.sort();
    Ok(entries)
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

fn sanitize_name(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| {
            if ch.is_ascii() && (ch.is_alphanumeric() || matches!(ch, '-' | '_')) {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if cleaned.is_empty() {
        "task".to_string()
    } else {
        cleaned
    }
}

fn file_name(path: &Path) -> &std::ffi::OsStr {
    path.file_name().unwrap_or_default()
}

fn file_name_string(path: &Path) -> String {
    file_name(path).to_string_lossy().into_owned()
}

fn trial_dir_name(path: &Path) -> String {
    file_name_string(path)
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_run_sort_keeps_revision_order() {
        let names = ["revision-10", "proposal", "revision", "revision-2"];
        let mut paths = names.iter().map(PathBuf::from).collect::<Vec<_>>();
        paths.sort_by_key(|path| worker_dir_sort_key(path));
        assert_eq!(
            paths
                .iter()
                .map(|path| file_name_string(path))
                .collect::<Vec<_>>(),
            ["proposal", "revision", "revision-2", "revision-10"]
        );
    }

    #[test]
    fn tool_proxy_run_sort_uses_timestamp_across_task_kinds() {
        let root = PathBuf::from("agent/tool-proxy-runs");
        let mut paths = [
            "app-123/cli/tool-20260712T103351-233Z",
            "app-123/cli/ask-20260712T103644-071Z",
            "app-123/cli/tool-20260712T102815-806Z",
        ]
        .iter()
        .map(|path| root.join(path))
        .collect::<Vec<_>>();

        paths.sort_by_key(|path| tool_proxy_run_sort_key(path, &root));

        assert_eq!(
            paths
                .iter()
                .map(|path| file_name_string(path))
                .collect::<Vec<_>>(),
            [
                "tool-20260712T102815-806Z",
                "tool-20260712T103351-233Z",
                "ask-20260712T103644-071Z",
            ]
        );
    }

    #[test]
    fn sanitize_name_matches_python_shape() {
        assert_eq!(sanitize_name("pkg/task name.json"), "pkg-task-name-json");
        assert_eq!(sanitize_name("***"), "task");
    }

    #[test]
    fn normalizes_minimal_pier_job() {
        let temp = tempfile::tempdir().unwrap();
        let pool = temp.path().join("job-output");
        let job_dir = temp.path().join("pier-jobs/job-1");
        let trial_dir = job_dir.join("trial-a");
        fs::create_dir_all(trial_dir.join("agent/worker-runs/proposal/logs")).unwrap();
        fs::create_dir_all(
            trial_dir.join("agent/tool-proxy-runs/app-123/cli/tool-20260712T000000Z/logs"),
        )
        .unwrap();
        fs::create_dir_all(
            trial_dir.join("agent/tool-proxy-runs/app-123/cli/tool-20260712T000000Z/tool-output"),
        )
        .unwrap();
        fs::create_dir_all(trial_dir.join("artifacts")).unwrap();
        fs::write(
            trial_dir.join("config.json"),
            r#"{"task":{"path":"/tasks/mashumaro-flattened-dataclass-fields"}}"#,
        )
        .unwrap();
        fs::write(trial_dir.join("agent/supervisor-feedback.jsonl"), "{}\n").unwrap();
        fs::write(
            trial_dir.join("agent/worker-runs/proposal/logs/opencode.stdout.txt"),
            "thinking\n",
        )
        .unwrap();
        fs::write(
            trial_dir.join("agent/worker-runs/proposal/opencode-instructions.md"),
            "rendered prompt\n",
        )
        .unwrap();
        fs::write(
            trial_dir.join(
                "agent/tool-proxy-runs/app-123/cli/tool-20260712T000000Z/opencode-instructions.md",
            ),
            "tool prompt\n",
        )
        .unwrap();
        fs::write(
            trial_dir.join("agent/tool-proxy-runs/app-123/cli/tool-20260712T000000Z/metrics.json"),
            r#"{"status":"success","changed_file_count":2,"changed_line_count":17}"#,
        )
        .unwrap();
        fs::write(
            trial_dir
                .join("agent/tool-proxy-runs/app-123/cli/tool-20260712T000000Z/tool-task.json"),
            r#"{"context":{"worker_role":"diff_review","original_command":"git diff --stat"}}"#,
        )
        .unwrap();
        fs::write(
            trial_dir.join(
                "agent/tool-proxy-runs/app-123/cli/tool-20260712T000000Z/logs/opencode.stdout.txt",
            ),
            "tool output\n",
        )
        .unwrap();
        fs::write(
            trial_dir.join(
                "agent/tool-proxy-runs/app-123/cli/tool-20260712T000000Z/tool-output/output.txt",
            ),
            "full output\n",
        )
        .unwrap();
        fs::write(trial_dir.join("artifacts/model.patch"), "diff\n").unwrap();

        let bundles = normalize_pier_job(&pool, &job_dir, &[]).unwrap();
        assert_eq!(bundles.len(), 1);
        let bundle_dir = pool.join("tasks/mashumaro-flattened-dataclass-fields");
        assert!(bundle_dir.join("artifact-manifest.json").exists());
        assert!(bundle_dir.join("agent/supervisor-feedback.jsonl").exists());
        assert!(
            bundle_dir
                .join("agent/worker-runs/proposal/opencode-instructions.md")
                .exists()
        );
        assert!(bundle_dir.join("artifacts/model.patch").exists());
        let manifest = load_json(&bundle_dir.join("artifact-manifest.json"));
        assert_eq!(
            manifest["worker_runs"][0]["artifact_sizes"]["opencode-instructions.md"],
            json!(16)
        );
        assert_eq!(
            manifest["tool_proxy_runs"][0]["path"],
            json!("agent/tool-proxy-runs/app-123/cli/tool-20260712T000000Z")
        );
        assert_eq!(manifest["tool_proxy_runs"][0]["kind"], json!("command"));
        assert_eq!(manifest["tool_proxy_runs"][0]["completed"], json!(true));
        assert_eq!(manifest["tool_proxy_runs"][0]["status"], json!("success"));
        assert_eq!(
            manifest["tool_proxy_runs"][0]["changed_file_count"],
            json!(2)
        );
        assert_eq!(
            manifest["tool_proxy_runs"][0]["changed_line_count"],
            json!(17)
        );
        assert_eq!(
            manifest["tool_proxy_runs"][0]["worker_role"],
            json!("diff_review")
        );
        assert_eq!(
            manifest["tool_proxy_runs"][0]["artifact_sizes"]["logs/opencode.stdout.txt"],
            json!(12)
        );
        assert_eq!(
            manifest["tool_proxy_runs"][0]["artifact_sizes"]["tool-output"],
            json!(12)
        );
        assert_eq!(
            manifest["important_artifacts"]["tool_proxy_runs"],
            json!("agent/tool-proxy-runs")
        );
        assert!(pool.join("artifact-bundles.json").exists());
        assert_eq!(
            fs::read_to_string(pool.parent().unwrap().join("latest.txt")).unwrap(),
            "job-output\n"
        );
    }
}
