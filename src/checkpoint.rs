use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    FrontierFeedbackTurn, PatchStats, file_len, get_bool, patch_stats, read_json_file,
    write_pretty_json,
};

pub(crate) const PATCH_COMPARISON: &str = "patch-comparison.json";
pub(crate) const PREVIOUS_WORKTREE_PATCH: &str = "previous-worktree.patch";

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
    /// Whether the heuristic thinks the revision degraded the candidate.
    pub(crate) degradation_detected: bool,
    /// Concrete reasons for the degradation signal.
    pub(crate) reasons: Vec<String>,
    /// Guidance included for the next supervisor review.
    pub(crate) supervisor_guidance: String,
}

/// Write checkpoint artifacts for a revision run and return the comparison.
pub(crate) fn write_patch_checkpoint_comparison(
    previous_run_dir: &Path,
    current_run_dir: &Path,
    decision: &FrontierFeedbackTurn,
) -> Result<PatchCheckpointComparison> {
    let previous_patch_path = previous_run_dir.join("worktree.patch");
    let previous_copy_path = current_run_dir.join(PREVIOUS_WORKTREE_PATCH);
    fs::copy(&previous_patch_path, &previous_copy_path).with_context(|| {
        format!(
            "failed to copy previous patch {} to {}",
            previous_patch_path.display(),
            previous_copy_path.display()
        )
    })?;

    let previous_patch = fs::read_to_string(&previous_copy_path)
        .with_context(|| format!("failed to read {}", previous_copy_path.display()))?;
    let current_patch_path = current_run_dir.join("worktree.patch");
    let current_patch = fs::read_to_string(&current_patch_path)
        .with_context(|| format!("failed to read {}", current_patch_path.display()))?;
    let latest_delta_path = current_run_dir.join("changes.patch");

    let previous_stats = patch_stats(&previous_patch);
    let current_stats = patch_stats(&current_patch);
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

    let mut reasons = Vec::new();
    if previous_patch_bytes > 0 && current_patch_bytes == 0 {
        reasons.push("current patch is empty after a non-empty previous patch".to_string());
    }
    if !previous_files.is_empty() && current_files.is_empty() {
        reasons.push("current patch no longer changes any files".to_string());
    }
    if !lost_changed_files.is_empty() && current_files.len() < previous_files.len() {
        reasons.push(format!(
            "current patch lost changed file(s): {}",
            lost_changed_files.join(", ")
        ));
    }
    if !lost_focus_files.is_empty() {
        reasons.push(format!(
            "current patch lost focused file(s): {}",
            lost_focus_files.join(", ")
        ));
    }
    if !focus_files.is_empty()
        && previous_files.iter().any(|file| focus_files.contains(file))
        && !current_files.iter().any(|file| focus_files.contains(file))
    {
        reasons.push("current patch no longer touches any focused files".to_string());
    }
    if latest_delta_bytes == 0 && current_patch_bytes < previous_patch_bytes {
        reasons.push(
            "latest worker delta is empty while accumulated patch shrank from previous candidate"
                .to_string(),
        );
    }

    let degradation_detected = !reasons.is_empty();
    let supervisor_guidance = if degradation_detected {
        "A worker revision may have degraded the candidate. Choose patch_decision explicitly: accept_current/revise_current if the current patch is better, or revise_previous if the previous patch should be preserved and the worker should recover from it."
    } else {
        "No patch degradation heuristic fired. Review the current worktree.patch normally."
    }
    .to_string();

    let comparison = PatchCheckpointComparison {
        previous_patch_artifact: PREVIOUS_WORKTREE_PATCH.to_string(),
        current_patch_artifact: "worktree.patch".to_string(),
        previous_patch_bytes,
        current_patch_bytes,
        latest_delta_bytes,
        previous_stats,
        current_stats,
        lost_changed_files,
        gained_changed_files,
        preserved_changed_files,
        focus_files: focus_files.into_iter().collect(),
        lost_focus_files,
        current_non_focus_files,
        degradation_detected,
        reasons,
        supervisor_guidance,
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
    let comparison = read_json_file(&comparison_path)?;
    if get_bool(&comparison, "degradation_detected").unwrap_or(false) {
        let previous_patch = run_dir.join(PREVIOUS_WORKTREE_PATCH);
        if previous_patch.exists() {
            artifact_paths.push(previous_patch);
        }
    }
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
            "degradation_detected": get_bool(&comparison, "degradation_detected").unwrap_or(false),
            "reasons": comparison.get("reasons").cloned().unwrap_or_else(|| json!([])),
            "previous_patch_bytes": comparison.get("previous_patch_bytes").cloned().unwrap_or_else(|| json!(0)),
            "current_patch_bytes": comparison.get("current_patch_bytes").cloned().unwrap_or_else(|| json!(0)),
            "latest_delta_bytes": comparison.get("latest_delta_bytes").cloned().unwrap_or_else(|| json!(0)),
            "lost_changed_files": comparison.get("lost_changed_files").cloned().unwrap_or_else(|| json!([])),
            "lost_focus_files": comparison.get("lost_focus_files").cloned().unwrap_or_else(|| json!([])),
        }));
    }
    Ok(json!({
        "count": items.len(),
        "degradation_count": items
            .iter()
            .filter(|item| get_bool(item, "degradation_detected").unwrap_or(false))
            .count(),
        "items": items
    }))
}
