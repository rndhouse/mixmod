use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{Value, json};

use crate::{
    CHANGES_PATCH, METRICS_JSON, PATCH_COMPARISON, SUPERVISION_LOOP_SUMMARY_JSON,
    SUPERVISOR_CONTROL_LOG, TASK_JSON, WORKER_BRIEF_JSON, WORKTREE_PATCH, file_len, get_bool,
    get_str, get_string_array, get_u64, patch_stats, read_json_file, write_pretty_json,
};

/// Write compact observed worker-loop telemetry for the next supervisor review.
pub(crate) fn write_supervision_loop_summary(
    default_dir: &Path,
    worker_run_dirs: &[PathBuf],
) -> Result<PathBuf> {
    let turns = worker_run_dirs
        .iter()
        .enumerate()
        .map(|(index, run_dir)| summarize_worker_turn(default_dir, index, run_dir))
        .collect::<Vec<_>>();

    let completed_worker_turn_count = turns
        .iter()
        .filter(|turn| get_bool(turn, "completed").unwrap_or(false))
        .count();
    let small_patch_slice_turn_count = turns
        .iter()
        .filter(|turn| get_str(turn, "worker_turn_shape") == Some("small_patch_slice"))
        .count();
    let small_patch_slice_nonempty_delta_count = turns
        .iter()
        .filter(|turn| {
            get_str(turn, "worker_turn_shape") == Some("small_patch_slice")
                && get_u64(turn, "latest_delta_bytes").unwrap_or(0) > 0
        })
        .count();
    let small_patch_slice_nonempty_delta_streak = turns
        .iter()
        .rev()
        .take_while(|turn| {
            get_str(turn, "worker_turn_shape") == Some("small_patch_slice")
                && get_u64(turn, "latest_delta_bytes").unwrap_or(0) > 0
                && get_u64(turn, "context_overflow_count").unwrap_or(0) == 0
        })
        .count();
    let context_overflow_total = turns
        .iter()
        .map(|turn| get_u64(turn, "context_overflow_count").unwrap_or(0))
        .sum::<u64>();
    let supervisor_control_count = turns
        .iter()
        .map(|turn| get_u64(turn, "supervisor_control_count").unwrap_or(0))
        .sum::<u64>();
    let worker_session_token_peak_max = turns
        .iter()
        .filter_map(|turn| get_u64(turn, "worker_session_token_peak"))
        .max();

    let summary = json!({
        "kind": "supervision-loop-summary",
        "recorded_at": chrono::Utc::now().to_rfc3339(),
        "purpose": "Observed worker-loop telemetry for supervisor review; Mixmod does not choose the next plan from this artifact.",
        "worker_turn_count": turns.len(),
        "completed_worker_turn_count": completed_worker_turn_count,
        "small_patch_slice_turn_count": small_patch_slice_turn_count,
        "small_patch_slice_nonempty_delta_count": small_patch_slice_nonempty_delta_count,
        "small_patch_slice_nonempty_delta_streak": small_patch_slice_nonempty_delta_streak,
        "context_overflow_total": context_overflow_total,
        "supervisor_control_count": supervisor_control_count,
        "worker_session_token_peak_max": worker_session_token_peak_max,
        "turns": turns,
    });

    let path = default_dir.join(SUPERVISION_LOOP_SUMMARY_JSON);
    write_pretty_json(&path, &summary, "supervision loop summary")?;
    Ok(path)
}

fn summarize_worker_turn(default_dir: &Path, index: usize, run_dir: &Path) -> Value {
    let metrics_path = run_dir.join(METRICS_JSON);
    let metrics = read_json_file(&metrics_path).unwrap_or_else(|_| json!({}));
    let handoff = handoff_context(default_dir, index, run_dir);
    let changes_patch = read_to_string_lossy(&run_dir.join(CHANGES_PATCH));
    let worktree_patch = read_to_string_lossy(&run_dir.join(WORKTREE_PATCH));
    let latest_delta_stats = patch_stats(&changes_patch);
    let accumulated_patch_stats = patch_stats(&worktree_patch);
    let patch_comparison = read_json_file(&run_dir.join(PATCH_COMPARISON)).ok();
    let control_log_events = read_jsonl_values(&run_dir.join(SUPERVISOR_CONTROL_LOG));
    let supervisor_control_count_source = if !control_log_events.is_empty() {
        "control_log"
    } else if metrics_path.exists() {
        "metrics_json"
    } else {
        "none"
    };
    let supervisor_control_events = if control_log_events.is_empty() {
        metrics
            .get("supervisor_control_events")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    } else {
        control_log_events
    };
    let supervisor_control_actions = supervisor_control_events
        .iter()
        .filter_map(|event| get_str(event, "action").map(ToOwned::to_owned))
        .collect::<Vec<_>>();

    json!({
        "index": index + 1,
        "run_dir": run_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("worker-run"),
        "role": if index == 0 { "proposal" } else { "revision" },
        "supervisor_label": handoff.supervisor_label,
        "worker_turn_shape": handoff.worker_turn_shape,
        "turn_goal": handoff.turn_goal,
        "worker_mode": handoff.worker_mode,
        "patch_decision": handoff.patch_decision,
        "focus_files": handoff.focus_files,
        "completed": metrics_path.exists(),
        "worker_session_reused": get_bool(&metrics, "worker_session_reused"),
        "worker_session_token_peak": get_u64(&metrics, "worker_session_token_peak"),
        "context_overflow_count": get_u64(&metrics, "context_overflow_count").unwrap_or(0),
        "wall_clock_ms": get_u64(&metrics, "wall_clock_ms"),
        "latest_delta_bytes": file_len(&run_dir.join(CHANGES_PATCH)).unwrap_or(0),
        "latest_delta_stats": latest_delta_stats,
        "accumulated_patch_bytes": file_len(&run_dir.join(WORKTREE_PATCH)).unwrap_or(0),
        "accumulated_patch_stats": accumulated_patch_stats,
        "supervisor_control_count": supervisor_control_events.len(),
        "supervisor_control_count_source": supervisor_control_count_source,
        "supervisor_control_actions": supervisor_control_actions,
        "interrupted_by_supervisor": get_bool(&metrics, "interrupted_by_supervisor").unwrap_or(false),
        "patch_observations": patch_comparison
            .as_ref()
            .and_then(|value| value.get("observations").cloned())
            .unwrap_or_else(|| json!([])),
    })
}

#[derive(Default)]
struct HandoffContext {
    supervisor_label: Option<String>,
    worker_turn_shape: Option<String>,
    turn_goal: Option<String>,
    worker_mode: Option<String>,
    patch_decision: Option<String>,
    focus_files: Vec<String>,
}

fn handoff_context(default_dir: &Path, index: usize, run_dir: &Path) -> HandoffContext {
    if index == 0 {
        let brief = read_json_file(&default_dir.join(WORKER_BRIEF_JSON)).ok();
        return HandoffContext {
            supervisor_label: Some("worker-brief".to_string()),
            worker_turn_shape: brief
                .as_ref()
                .and_then(|value| get_str(value, "worker_turn_shape").map(ToOwned::to_owned)),
            turn_goal: brief
                .as_ref()
                .and_then(|value| get_str(value, "turn_goal").map(ToOwned::to_owned)),
            worker_mode: Some("initial".to_string()),
            patch_decision: None,
            focus_files: brief
                .as_ref()
                .map(|value| get_string_array(value, "files"))
                .unwrap_or_default(),
        };
    }

    let task = read_json_file(&run_dir.join(TASK_JSON))
        .ok()
        .or_else(|| read_json_file(&revision_task_path(default_dir, index)).ok());
    let revision = task.as_ref().and_then(|value| {
        value
            .pointer("/context/revision")
            .or_else(|| value.pointer("/context/revision_details"))
    });
    let supervisor_label = if index == 1 {
        "critique".to_string()
    } else {
        format!("critique-{index}")
    };
    HandoffContext {
        supervisor_label: Some(supervisor_label),
        worker_turn_shape: revision
            .and_then(|value| get_str(value, "worker_turn_shape"))
            .map(ToOwned::to_owned),
        turn_goal: revision
            .and_then(|value| get_str(value, "turn_goal"))
            .map(ToOwned::to_owned),
        worker_mode: revision
            .and_then(|value| get_str(value, "worker_mode"))
            .map(ToOwned::to_owned),
        patch_decision: revision
            .and_then(|value| get_str(value, "patch_decision"))
            .map(ToOwned::to_owned),
        focus_files: revision
            .map(|value| get_string_array(value, "repo_focus_files"))
            .filter(|files| !files.is_empty())
            .or_else(|| revision.map(|value| get_string_array(value, "focus_files")))
            .unwrap_or_default(),
    }
}

fn revision_task_path(default_dir: &Path, index: usize) -> PathBuf {
    if index == 1 {
        default_dir.join("revision-task.json")
    } else {
        default_dir.join(format!("revision-task-{index}.json"))
    }
}

fn read_to_string_lossy(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn read_jsonl_values(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::{atomic_write, supervisor_review_artifact_paths};

    #[test]
    fn loop_summary_counts_small_patch_delta_streak() {
        let temp = TempDir::new().unwrap();
        let default_dir = temp.path().join("default");
        let worker_root = temp.path().join("worker-runs");
        let proposal = worker_root.join("proposal");
        let revision = worker_root.join("revision");
        std::fs::create_dir_all(&proposal).unwrap();
        std::fs::create_dir_all(&revision).unwrap();
        std::fs::create_dir_all(&default_dir).unwrap();

        write_pretty_json(
            &default_dir.join(WORKER_BRIEF_JSON),
            &json!({
                "worker_turn_shape": "small_patch_slice",
                "turn_goal": "Seed option parsing",
                "files": ["src/lib.rs"]
            }),
            "worker brief",
        )
        .unwrap();
        write_pretty_json(
            &default_dir.join("revision-task.json"),
            &json!({
                "context": {
                    "revision": {
                        "worker_turn_shape": "small_patch_slice",
                        "turn_goal": "Wire option into runtime",
                        "worker_mode": "continue",
                        "patch_decision": "revise_current",
                        "repo_focus_files": ["src/lib.rs"]
                    }
                }
            }),
            "revision task",
        )
        .unwrap();

        write_worker_metrics(&proposal, false, 1200, 0, 4);
        write_worker_metrics(&revision, true, 1800, 0, 8);
        write_patch(&proposal, "src/lib.rs", "+seed\n");
        write_patch(&revision, "src/lib.rs", "+runtime\n");

        let summary_path =
            write_supervision_loop_summary(&default_dir, &[proposal.clone(), revision.clone()])
                .unwrap();
        let summary = read_json_file(&summary_path).unwrap();

        assert_eq!(get_u64(&summary, "worker_turn_count"), Some(2));
        assert_eq!(
            get_u64(&summary, "small_patch_slice_nonempty_delta_streak"),
            Some(2)
        );
        assert_eq!(
            get_u64(&summary, "worker_session_token_peak_max"),
            Some(1800)
        );
        assert_eq!(
            summary
                .get("turns")
                .and_then(Value::as_array)
                .and_then(|turns| turns.get(1))
                .and_then(|turn| get_str(turn, "turn_goal")),
            Some("Wire option into runtime")
        );
    }

    #[test]
    fn supervisor_review_includes_loop_summary_when_present() {
        let temp = TempDir::new().unwrap();
        let default_dir = temp.path().join("default");
        let worker_dir = temp.path().join("worker");
        std::fs::create_dir_all(&default_dir).unwrap();
        std::fs::create_dir_all(&worker_dir).unwrap();
        atomic_write(&default_dir.join(TASK_JSON), b"{}").unwrap();
        atomic_write(&default_dir.join(WORKER_BRIEF_JSON), b"{}").unwrap();
        atomic_write(&default_dir.join(SUPERVISION_LOOP_SUMMARY_JSON), b"{}").unwrap();
        atomic_write(&worker_dir.join(METRICS_JSON), b"{}").unwrap();

        let paths = supervisor_review_artifact_paths(&default_dir, &worker_dir)
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(paths.contains(&SUPERVISION_LOOP_SUMMARY_JSON.to_string()));
    }

    #[test]
    fn loop_summary_counts_control_log_for_incomplete_turn() {
        let temp = TempDir::new().unwrap();
        let default_dir = temp.path().join("default");
        let worker_root = temp.path().join("worker-runs");
        let proposal = worker_root.join("proposal");
        let revision = worker_root.join("revision");
        std::fs::create_dir_all(&proposal).unwrap();
        std::fs::create_dir_all(&revision).unwrap();
        std::fs::create_dir_all(&default_dir).unwrap();

        write_pretty_json(
            &default_dir.join(WORKER_BRIEF_JSON),
            &json!({
                "worker_turn_shape": "small_patch_slice",
                "turn_goal": "Seed API",
                "files": ["src/lib.rs"]
            }),
            "worker brief",
        )
        .unwrap();
        write_pretty_json(
            &default_dir.join("revision-task.json"),
            &json!({
                "context": {
                    "revision": {
                        "worker_turn_shape": "small_patch_slice",
                        "turn_goal": "Patch runtime branch",
                        "worker_mode": "context_focus",
                        "patch_decision": "revise_current",
                        "repo_focus_files": ["src/lib.rs"]
                    }
                }
            }),
            "revision task",
        )
        .unwrap();

        write_worker_metrics(&proposal, false, 1200, 0, 4);
        write_patch(&proposal, "src/lib.rs", "+seed\n");
        write_patch(&revision, "src/lib.rs", "+runtime\n");
        atomic_write(
            &revision.join(SUPERVISOR_CONTROL_LOG),
            br#"{"action":"interrupt_continue","risk":"Repeated reads with no repository delta"}
"#,
        )
        .unwrap();

        let summary_path =
            write_supervision_loop_summary(&default_dir, &[proposal.clone(), revision.clone()])
                .unwrap();
        let summary = read_json_file(&summary_path).unwrap();
        let turns = summary.get("turns").and_then(Value::as_array).unwrap();
        let revision_turn = turns.get(1).unwrap();

        assert_eq!(get_u64(&summary, "worker_turn_count"), Some(2));
        assert_eq!(get_u64(&summary, "completed_worker_turn_count"), Some(1));
        assert_eq!(get_u64(&summary, "supervisor_control_count"), Some(1));
        assert_eq!(get_bool(revision_turn, "completed"), Some(false));
        assert_eq!(
            get_str(revision_turn, "supervisor_control_count_source"),
            Some("control_log")
        );
        assert_eq!(
            revision_turn
                .get("supervisor_control_actions")
                .and_then(Value::as_array)
                .and_then(|actions| actions.first())
                .and_then(Value::as_str),
            Some("interrupt_continue")
        );
    }

    fn write_worker_metrics(
        run_dir: &Path,
        worker_session_reused: bool,
        worker_session_token_peak: u64,
        context_overflow_count: u64,
        revision_delta_bytes: u64,
    ) {
        write_pretty_json(
            &run_dir.join(METRICS_JSON),
            &json!({
                "worker_session_reused": worker_session_reused,
                "worker_session_token_peak": worker_session_token_peak,
                "context_overflow_count": context_overflow_count,
                "wall_clock_ms": 100,
                "revision_delta_bytes": revision_delta_bytes,
                "supervisor_control_events": []
            }),
            "metrics",
        )
        .unwrap();
    }

    fn write_patch(run_dir: &Path, file: &str, line: &str) {
        let patch = format!(
            "diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -1,1 +1,2 @@\n old\n{line}"
        );
        atomic_write(&run_dir.join(CHANGES_PATCH), patch.as_bytes()).unwrap();
        atomic_write(&run_dir.join(WORKTREE_PATCH), patch.as_bytes()).unwrap();
    }
}
