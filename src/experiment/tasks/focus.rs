use std::ffi::OsStr;
use std::path::Path;

use crate::is_static_mixmod_artifact_name;

pub(super) fn split_worker_focus_files(
    work_dir: &Path,
    default_dir: &Path,
    requested: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut repo_files = Vec::new();
    let mut artifact_refs = Vec::new();
    for raw in requested {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        match classify_worker_focus_file(work_dir, default_dir, trimmed) {
            WorkerFocusFile::Repo(path) => push_unique(&mut repo_files, path),
            WorkerFocusFile::Artifact(path) => push_unique(&mut artifact_refs, path),
        }
    }
    (repo_files, artifact_refs)
}

enum WorkerFocusFile {
    Repo(String),
    Artifact(String),
}

fn classify_worker_focus_file(work_dir: &Path, default_dir: &Path, raw: &str) -> WorkerFocusFile {
    let normalized = raw.trim_start_matches("./").replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        if let Ok(relative) = path.strip_prefix(work_dir) {
            let relative = path_to_repo_string(relative);
            if is_artifact_focus_ref(&relative) {
                WorkerFocusFile::Artifact(normalized)
            } else {
                WorkerFocusFile::Repo(relative)
            }
        } else {
            let _ = path.strip_prefix(default_dir);
            WorkerFocusFile::Artifact(normalized)
        }
    } else if normalized.starts_with("../")
        || normalized.contains("/../")
        || normalized.starts_with(".mixmod/")
        || normalized.starts_with(".codex/")
        || is_artifact_focus_ref(&normalized)
    {
        WorkerFocusFile::Artifact(normalized)
    } else {
        WorkerFocusFile::Repo(normalized)
    }
}

fn is_artifact_focus_ref(path: &str) -> bool {
    let file_name = Path::new(path)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(path);
    is_static_mixmod_artifact_name(file_name)
        || file_name == "revision-task.json"
        || (file_name.starts_with("revision-task-") && file_name.ends_with(".json"))
}

fn path_to_repo_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn push_unique(items: &mut Vec<String>, item: String) {
    if !items.contains(&item) {
        items.push(item);
    }
}
