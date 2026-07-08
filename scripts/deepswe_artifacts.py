"""Helpers for organizing DeepSWE Pier run artifacts."""

from __future__ import annotations

import json
import shutil
from pathlib import Path
from typing import Any


def normalize_pier_job(
    pool: Path,
    job_dir: Path,
    records: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Copy Pier trial artifacts into a stable per-task review layout."""
    if not job_dir.exists():
        return []

    copied_job_files = _copy_job_files(pool, job_dir)
    task_bundles = []
    used_names: set[str] = set()
    for trial_dir in _trial_dirs(job_dir):
        task_name = _task_name_from_trial(trial_dir)
        bundle_name = _unique_bundle_name(task_name, trial_dir.name, used_names)
        target = pool / "tasks" / bundle_name
        related_records = _records_for_trial(records, trial_dir)
        task_bundles.append(
            _copy_trial_bundle(
                trial_dir=trial_dir,
                target=target,
                task_name=task_name,
                related_records=related_records,
            )
        )

    manifest = {
        "kind": "deepswe-pier-artifact-bundles",
        "job_name": pool.name,
        "job_dir": str(job_dir),
        "tasks_dir": str(pool / "tasks"),
        "pier_job_files": copied_job_files,
        "task_bundles": task_bundles,
    }
    _write_json(pool / "artifact-bundles.json", manifest)
    _update_latest_pointer(pool)
    return task_bundles


def _copy_job_files(pool: Path, job_dir: Path) -> list[str]:
    target = pool / "pier-job"
    target.mkdir(parents=True, exist_ok=True)
    copied = []
    for source in sorted(path for path in job_dir.iterdir() if path.is_file()):
        copied_path = _copy_path(source, target / source.name)
        if copied_path:
            copied.append(_relative_to(pool, copied_path))
    return copied


def _trial_dirs(job_dir: Path) -> list[Path]:
    return sorted(
        path
        for path in job_dir.iterdir()
        if path.is_dir() and ((path / "agent").exists() or (path / "config.json").exists())
    )


def _task_name_from_trial(trial_dir: Path) -> str:
    config = _load_json(trial_dir / "config.json")
    task = config.get("task")
    if isinstance(task, dict):
        task_path = task.get("path")
        if isinstance(task_path, str) and task_path:
            return Path(task_path).name
        task_name = task.get("name")
        if isinstance(task_name, str) and task_name:
            return task_name.rsplit("/", 1)[-1]

    result = _load_json(trial_dir / "result.json")
    task_id = result.get("task_id")
    if isinstance(task_id, dict):
        task_path = task_id.get("path")
        if isinstance(task_path, str) and task_path:
            return Path(task_path).name
    task_name = result.get("task_name")
    if isinstance(task_name, str) and task_name:
        return task_name.rsplit("/", 1)[-1]

    if "__" in trial_dir.name:
        return trial_dir.name.split("__", 1)[0]
    return trial_dir.name


def _unique_bundle_name(task_name: str, trial_name: str, used_names: set[str]) -> str:
    base = _sanitize_name(task_name)
    if base not in used_names:
        used_names.add(base)
        return base
    fallback = _sanitize_name(trial_name)
    if fallback not in used_names:
        used_names.add(fallback)
        return fallback
    index = 2
    while f"{fallback}-{index}" in used_names:
        index += 1
    unique = f"{fallback}-{index}"
    used_names.add(unique)
    return unique


def _copy_trial_bundle(
    trial_dir: Path,
    target: Path,
    task_name: str,
    related_records: list[dict[str, Any]],
) -> dict[str, Any]:
    if target.exists() or target.is_symlink():
        _remove_path(target)
    target.mkdir(parents=True, exist_ok=True)

    copied: dict[str, str] = {}
    for name in ["agent", "artifacts", "verifier"]:
        copied_path = _copy_path(trial_dir / name, target / name)
        if copied_path:
            copied[name] = _relative_to(target, copied_path)

    pier_dir = target / "pier"
    for source in sorted(path for path in trial_dir.iterdir() if path.is_file()):
        copied_path = _copy_path(source, pier_dir / source.name)
        if copied_path:
            copied[f"pier/{source.name}"] = _relative_to(target, copied_path)

    manifest = {
        "kind": "deepswe-task-artifact-bundle",
        "task": task_name,
        "trial_name": trial_dir.name,
        "source_trial_dir": str(trial_dir),
        "bundle_dir": str(target),
        "copied": copied,
        "records": related_records,
        "important_artifacts": _important_artifacts(target),
        "worker_runs": _worker_runs(target / "agent" / "worker-runs"),
    }
    _write_json(target / "artifact-manifest.json", manifest)
    return {
        "task": task_name,
        "trial_name": trial_dir.name,
        "bundle_dir": str(target),
        "manifest": str(target / "artifact-manifest.json"),
        "records": related_records,
        "worker_run_count": len(manifest["worker_runs"]),
        "latest_worker_run": (
            manifest["worker_runs"][-1]["name"] if manifest["worker_runs"] else None
        ),
    }


def _records_for_trial(
    records: list[dict[str, Any]],
    trial_dir: Path,
) -> list[dict[str, Any]]:
    related = []
    trial_prefix = str(trial_dir.resolve())
    for record in records:
        paths = [
            value
            for key, value in record.items()
            if key.endswith("_json") or key in {"test_stdout", "reward_json"}
        ]
        if any(_path_is_under(value, trial_prefix) for value in paths):
            related.append(record)
    return related


def _path_is_under(value: object, resolved_parent: str) -> bool:
    if not isinstance(value, str) or not value:
        return False
    try:
        Path(value).resolve().relative_to(resolved_parent)
    except (OSError, ValueError):
        return False
    return True


def _important_artifacts(target: Path) -> dict[str, str]:
    names = {
        "supervisor_feedback": target / "agent" / "supervisor-feedback.jsonl",
        "supervision_loop_summary": target / "agent" / "supervision-loop-summary.json",
        "worker_brief": target / "agent" / "worker-brief.json",
        "worker_task": target / "agent" / "worker-task.json",
        "mixmod_summary": target / "agent" / "mixmod-summary.json",
        "mixmod_metrics": target / "agent" / "mixmod-metrics.json",
        "mixmod_final_patch": target / "agent" / "mixmod-final.patch",
        "model_patch": target / "artifacts" / "model.patch",
        "git_status": target / "artifacts" / "git-status.txt",
    }
    return {
        key: _relative_to(target, path)
        for key, path in names.items()
        if path.exists()
    }


def _worker_runs(worker_root: Path) -> list[dict[str, Any]]:
    if not worker_root.exists():
        return []
    runs = []
    for worker_dir in sorted(
        (path for path in worker_root.iterdir() if path.is_dir()),
        key=_worker_dir_sort_key,
    ):
        artifact_sizes = {}
        for name in [
            "reasoning-trace.jsonl",
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
        ]:
            path = worker_dir / name
            if path.exists():
                artifact_sizes[name] = path.stat().st_size
        runs.append(
            {
                "name": worker_dir.name,
                "path": _relative_to(worker_root.parent.parent, worker_dir),
                "artifact_sizes": artifact_sizes,
            }
        )
    return runs


def _worker_dir_sort_key(path: Path) -> tuple[int, int, str]:
    name = path.name
    if name == "proposal":
        return (0, 0, name)
    if name == "revision":
        return (1, 1, name)
    if name.startswith("revision-"):
        suffix = name.rsplit("-", 1)[-1]
        if suffix.isdigit():
            return (1, int(suffix), name)
    return (2, 0, name)


def _copy_path(source: Path, target: Path) -> Path | None:
    if not source.exists():
        return None
    if target.exists() or target.is_symlink():
        _remove_path(target)
    target.parent.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        shutil.copytree(source, target, symlinks=True)
    else:
        shutil.copy2(source, target)
    return target


def _remove_path(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)


def _update_latest_pointer(pool: Path) -> None:
    latest = pool.parent / "latest"
    latest_txt = pool.parent / "latest.txt"
    latest_txt.write_text(pool.name + "\n")
    if latest.exists() and not latest.is_symlink() and latest.is_dir():
        return
    try:
        if latest.exists() or latest.is_symlink():
            latest.unlink()
        latest.symlink_to(pool.name, target_is_directory=True)
    except OSError:
        pass


def _load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return value if isinstance(value, dict) else {}


def _write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def _relative_to(root: Path, path: Path) -> str:
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def _sanitize_name(value: str) -> str:
    cleaned = "".join(
        ch if ch.isascii() and (ch.isalnum() or ch in "-_") else "-" for ch in value
    ).strip("-")
    return cleaned or "task"
