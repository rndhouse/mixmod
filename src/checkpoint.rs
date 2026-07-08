use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    CHANGES_PATCH, PATCH_COMPARISON, PATCH_ROLLBACK_JSON, PREVIOUS_WORKTREE_PATCH, PatchStats,
    ROLLBACK_CURRENT_PATCH, ROLLBACK_RESTORED_PATCH, SupervisorFeedbackTurn, TASK_JSON,
    WORKTREE_PATCH, atomic_write, file_len, git_diff_with_untracked, patch_stats, read_json_file,
    write_pretty_json,
};

/// Summary of a worker revision compared with the previous candidate patch.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PatchCheckpointComparison {
    /// Relative artifact name containing the previous candidate patch.
    pub(crate) previous_patch_artifact: String,
    /// Relative artifact name containing the current accumulated patch.
    pub(crate) current_patch_artifact: String,
    /// Previous candidate patch size in bytes.
    pub(crate) previous_patch_bytes: u64,
    /// Current accumulated patch size in bytes.
    pub(crate) current_patch_bytes: u64,
    /// Latest worker-turn delta size in bytes.
    pub(crate) latest_delta_bytes: u64,
    /// Patch stats for the latest worker-turn delta only.
    pub(crate) latest_delta_stats: PatchStats,
    /// Patch stats for the previous candidate.
    pub(crate) previous_stats: PatchStats,
    /// Patch stats for the current candidate.
    pub(crate) current_stats: PatchStats,
    /// Files that were changed before this revision but are no longer changed.
    pub(crate) lost_changed_files: Vec<String>,
    /// Files newly changed by this revision.
    pub(crate) gained_changed_files: Vec<String>,
    /// Files changed in both previous and current candidates.
    pub(crate) preserved_changed_files: Vec<String>,
    /// Focus files from the supervisor decision that triggered this revision.
    pub(crate) focus_files: Vec<String>,
    /// Focus files that were changed before the revision but not after it.
    pub(crate) lost_focus_files: Vec<String>,
    /// Current changed files that are outside the requested focus set.
    pub(crate) current_non_focus_files: Vec<String>,
    /// Neutral observations about structural differences between candidates.
    pub(crate) observations: Vec<String>,
}

/// Receipt for a filesystem rollback to a previous candidate patch.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PatchRollbackReceipt {
    /// Rollback outcome.
    pub(crate) status: String,
    /// Artifact that supplied the restored candidate patch.
    pub(crate) previous_patch_artifact: String,
    /// Artifact containing the discarded current candidate patch.
    pub(crate) discarded_patch_artifact: String,
    /// Artifact containing the patch captured after restore.
    pub(crate) restored_patch_artifact: String,
    /// Previous candidate patch size in bytes.
    pub(crate) previous_patch_bytes: u64,
    /// Discarded current patch size in bytes.
    pub(crate) discarded_patch_bytes: u64,
    /// Restored patch size in bytes.
    pub(crate) restored_patch_bytes: u64,
}

/// Restore the target worktree to the previous checkpoint patch.
pub(crate) fn restore_previous_patch_checkpoint(
    root: &Path,
    checkpoint_run_dir: &Path,
) -> Result<PatchRollbackReceipt> {
    let previous_patch_path = checkpoint_run_dir.join(PREVIOUS_WORKTREE_PATCH);
    let previous_patch = fs::read_to_string(&previous_patch_path)
        .with_context(|| format!("failed to read {}", previous_patch_path.display()))?;
    let current_patch = git_diff_with_untracked(root)
        .with_context(|| format!("failed to capture current diff in {}", root.display()))?;

    let current_patch_path = checkpoint_run_dir.join(ROLLBACK_CURRENT_PATCH);
    atomic_write(&current_patch_path, current_patch.as_bytes()).with_context(|| {
        format!(
            "failed to write rollback current patch {}",
            current_patch_path.display()
        )
    })?;

    reset_worktree(root).context("failed to reset worktree before checkpoint restore")?;
    clean_worktree(root).context("failed to clean worktree before checkpoint restore")?;
    if let Err(error) = apply_patch_if_nonempty(
        root,
        previous_patch_path.to_string_lossy().as_ref(),
        &previous_patch,
    ) {
        let _ = reset_worktree(root);
        let _ = clean_worktree(root);
        let _ = apply_patch_if_nonempty(
            root,
            current_patch_path.to_string_lossy().as_ref(),
            &current_patch,
        );
        return Err(error).with_context(|| {
            format!(
                "failed to restore previous checkpoint from {}",
                previous_patch_path.display()
            )
        });
    }

    let restored_patch = git_diff_with_untracked(root)
        .with_context(|| format!("failed to capture restored diff in {}", root.display()))?;
    let restored_patch_path = checkpoint_run_dir.join(ROLLBACK_RESTORED_PATCH);
    atomic_write(&restored_patch_path, restored_patch.as_bytes()).with_context(|| {
        format!(
            "failed to write rollback restored patch {}",
            restored_patch_path.display()
        )
    })?;
    if restored_patch.trim() != previous_patch.trim() {
        let _ = reset_worktree(root);
        let _ = clean_worktree(root);
        let _ = apply_patch_if_nonempty(
            root,
            current_patch_path.to_string_lossy().as_ref(),
            &current_patch,
        );
        bail!(
            "checkpoint restore verification failed: restored diff does not match {}",
            previous_patch_path.display()
        );
    }

    let receipt = PatchRollbackReceipt {
        status: "restored".to_string(),
        previous_patch_artifact: PREVIOUS_WORKTREE_PATCH.to_string(),
        discarded_patch_artifact: ROLLBACK_CURRENT_PATCH.to_string(),
        restored_patch_artifact: ROLLBACK_RESTORED_PATCH.to_string(),
        previous_patch_bytes: previous_patch.len() as u64,
        discarded_patch_bytes: current_patch.len() as u64,
        restored_patch_bytes: restored_patch.len() as u64,
    };
    write_pretty_json(
        &checkpoint_run_dir.join(PATCH_ROLLBACK_JSON),
        &receipt,
        "patch rollback receipt",
    )?;
    Ok(receipt)
}

fn reset_worktree(root: &Path) -> Result<()> {
    run_git(root, &["reset", "--hard", "HEAD"])
}

fn clean_worktree(root: &Path) -> Result<()> {
    run_git(
        root,
        &[
            "clean",
            "-fd",
            "-e",
            ".mixmod",
            "-e",
            ".mixmod/**",
            "-e",
            ".codex",
            "-e",
            ".codex/**",
            "-e",
            TASK_JSON,
        ],
    )
}

fn apply_patch_if_nonempty(root: &Path, patch_label: &str, patch: &str) -> Result<()> {
    if patch.trim().is_empty() {
        return Ok(());
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("apply")
        .arg("--binary")
        .arg("--whitespace=nowarn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run git apply in {}", root.display()))?;
    child
        .stdin
        .take()
        .context("failed to open git apply stdin")?
        .write_all(patch.as_bytes())
        .context("failed to write patch to git apply stdin")?;
    let output = child
        .wait_with_output()
        .context("failed to wait for git apply")?;
    if !output.status.success() {
        bail!(
            "git apply {} failed in {}: {}",
            patch_label,
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn run_git(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {} in {}", args.join(" "), root.display()))?;
    if !output.status.success() {
        bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Write checkpoint artifacts for a revision run and return the comparison.
#[cfg(test)]
pub(crate) fn write_patch_checkpoint_comparison(
    previous_run_dir: &Path,
    current_run_dir: &Path,
    decision: &SupervisorFeedbackTurn,
) -> Result<PatchCheckpointComparison> {
    write_patch_checkpoint_comparison_from_patch(
        &previous_run_dir.join(WORKTREE_PATCH),
        current_run_dir,
        decision,
    )
}

/// Write checkpoint artifacts using an explicit previous patch source.
pub(crate) fn write_patch_checkpoint_comparison_from_patch(
    previous_patch_path: &Path,
    current_run_dir: &Path,
    decision: &SupervisorFeedbackTurn,
) -> Result<PatchCheckpointComparison> {
    let previous_copy_path = current_run_dir.join(PREVIOUS_WORKTREE_PATCH);
    fs::copy(previous_patch_path, &previous_copy_path).with_context(|| {
        format!(
            "failed to copy previous patch {} to {}",
            previous_patch_path.display(),
            previous_copy_path.display()
        )
    })?;

    let previous_patch = fs::read_to_string(&previous_copy_path)
        .with_context(|| format!("failed to read {}", previous_copy_path.display()))?;
    let current_patch_path = current_run_dir.join(WORKTREE_PATCH);
    let current_patch = fs::read_to_string(&current_patch_path)
        .with_context(|| format!("failed to read {}", current_patch_path.display()))?;
    let latest_delta_path = current_run_dir.join(CHANGES_PATCH);
    let latest_delta_patch = fs::read_to_string(&latest_delta_path).unwrap_or_default();

    let previous_stats = patch_stats(&previous_patch);
    let current_stats = patch_stats(&current_patch);
    let latest_delta_stats = patch_stats(&latest_delta_patch);
    let latest_delta_bytes = file_len(&latest_delta_path).unwrap_or(0);
    let previous_patch_bytes = previous_patch.len() as u64;
    let current_patch_bytes = current_patch.len() as u64;

    let previous_files = previous_stats
        .files
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let current_files = current_stats.files.iter().cloned().collect::<BTreeSet<_>>();
    let focus_files = decision
        .focus_files
        .iter()
        .map(|file| file.trim())
        .filter(|file| !file.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();

    let lost_changed_files = previous_files
        .difference(&current_files)
        .cloned()
        .collect::<Vec<_>>();
    let gained_changed_files = current_files
        .difference(&previous_files)
        .cloned()
        .collect::<Vec<_>>();
    let preserved_changed_files = previous_files
        .intersection(&current_files)
        .cloned()
        .collect::<Vec<_>>();
    let lost_focus_files = previous_files
        .intersection(&focus_files)
        .filter(|file| !current_files.contains(*file))
        .cloned()
        .collect::<Vec<_>>();
    let current_non_focus_files = if focus_files.is_empty() {
        Vec::new()
    } else {
        current_files
            .difference(&focus_files)
            .cloned()
            .collect::<Vec<_>>()
    };

    let mut observations = Vec::new();
    if previous_patch_bytes > 0 && current_patch_bytes == 0 {
        observations.push("current patch is empty after a non-empty previous patch".to_string());
    }
    if !previous_files.is_empty() && current_files.is_empty() {
        observations.push("current patch no longer changes any files".to_string());
    }
    if !lost_changed_files.is_empty() && current_files.len() < previous_files.len() {
        observations.push(format!(
            "current patch lost changed file(s): {}",
            lost_changed_files.join(", ")
        ));
    }
    if !lost_focus_files.is_empty() {
        observations.push(format!(
            "current patch lost focused file(s): {}",
            lost_focus_files.join(", ")
        ));
    }
    if !focus_files.is_empty()
        && previous_files.iter().any(|file| focus_files.contains(file))
        && !current_files.iter().any(|file| focus_files.contains(file))
    {
        observations.push("current patch no longer touches any focused files".to_string());
    }
    if latest_delta_bytes == 0 && current_patch_bytes < previous_patch_bytes {
        observations.push(
            "latest worker delta is empty while accumulated patch shrank from previous candidate"
                .to_string(),
        );
    }
    if decision.revision_handoff.is_small_patch_slice() {
        if latest_delta_stats.removed_lines > 25 {
            observations.push(format!(
                "small patch slice latest delta removed lines: {}",
                latest_delta_stats.removed_lines
            ));
        }
        if latest_delta_stats.changed_line_count > 80 {
            observations.push(format!(
                "small patch slice latest delta changed line count: {}",
                latest_delta_stats.changed_line_count
            ));
        }
        if latest_delta_stats.files.len() > 2 {
            observations.push(format!(
                "small patch slice latest delta changed files: {}",
                latest_delta_stats.files.join(", ")
            ));
        }
    }

    let comparison = PatchCheckpointComparison {
        previous_patch_artifact: PREVIOUS_WORKTREE_PATCH.to_string(),
        current_patch_artifact: WORKTREE_PATCH.to_string(),
        previous_patch_bytes,
        current_patch_bytes,
        latest_delta_bytes,
        latest_delta_stats,
        previous_stats,
        current_stats,
        lost_changed_files,
        gained_changed_files,
        preserved_changed_files,
        focus_files: focus_files.into_iter().collect(),
        lost_focus_files,
        current_non_focus_files,
        observations,
    };
    write_pretty_json(
        &current_run_dir.join(PATCH_COMPARISON),
        &comparison,
        "patch checkpoint comparison",
    )?;
    Ok(comparison)
}

/// Add checkpoint artifacts to the next supervisor review when present.
pub(crate) fn append_patch_checkpoint_artifacts(
    run_dir: &Path,
    artifact_paths: &mut Vec<PathBuf>,
) -> Result<()> {
    let comparison_path = run_dir.join(PATCH_COMPARISON);
    if !comparison_path.exists() {
        return Ok(());
    }
    artifact_paths.push(comparison_path.clone());
    Ok(())
}

/// Return compact checkpoint metrics for the final strategy metrics object.
pub(crate) fn patch_checkpoint_metrics(worker_run_dirs: &[PathBuf]) -> Result<serde_json::Value> {
    let mut items = Vec::new();
    for dir in worker_run_dirs {
        let path = dir.join(PATCH_COMPARISON);
        if !path.exists() {
            continue;
        }
        let comparison = read_json_file(&path)?;
        items.push(json!({
            "run_dir": dir.to_string_lossy(),
            "observations": comparison.get("observations").cloned().unwrap_or_else(|| json!([])),
            "previous_patch_bytes": comparison.get("previous_patch_bytes").cloned().unwrap_or_else(|| json!(0)),
            "current_patch_bytes": comparison.get("current_patch_bytes").cloned().unwrap_or_else(|| json!(0)),
            "latest_delta_bytes": comparison.get("latest_delta_bytes").cloned().unwrap_or_else(|| json!(0)),
            "latest_delta_stats": comparison.get("latest_delta_stats").cloned().unwrap_or_else(|| json!({})),
            "lost_changed_files": comparison.get("lost_changed_files").cloned().unwrap_or_else(|| json!([])),
            "lost_focus_files": comparison.get("lost_focus_files").cloned().unwrap_or_else(|| json!([])),
        }));
    }
    Ok(json!({
        "count": items.len(),
        "items": items
    }))
}
