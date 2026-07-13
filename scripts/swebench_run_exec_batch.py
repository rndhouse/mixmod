#!/usr/bin/env python3
"""Run a selected SWE-bench Lite batch through `mixmod exec`.

This script is benchmark glue. It creates fresh worktrees from the repo cache,
runs Mixmod with explicit supervisor/worker model choices, and optionally scores
each patch with the official SWE-bench evaluator.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


DEFAULT_MODEL_NAME = "mixmod-qwen36-current-10-v2"
DEFAULT_SUPERVISOR_MODEL = "gpt-5.5:high"
DEFAULT_WORKER_MODEL = "llama.cpp/qwen/qwen3.6-27b"
DEFAULT_POOL_NAME = "qwen36-current-10-v2"

SELECTED_INSTANCES = [
    "pytest-dev__pytest-11143",
    "scikit-learn__scikit-learn-13439",
    "sympy__sympy-20212",
    "django__django-12908",
    "pytest-dev__pytest-6116",
    "django__django-13447",
    "django__django-15814",
    "django__django-11179",
    "sympy__sympy-13480",
    "scikit-learn__scikit-learn-13584",
]

TASK_PREFIXES = [
    "swebench-current-default-v1-",
    "swebench-expect-patch-v1-",
    "swebench-explicit-gpt55-high-v1-",
    "swebench-codex-pass-pool-v1-",
]


@dataclass(frozen=True)
class InstanceTask:
    instance_id: str
    repo: str
    base_commit: str
    task_json: Path


def fnv1a64(value: bytes) -> int:
    hash_value = 0xCBF29CE484222325
    for byte in value:
        hash_value ^= byte
        hash_value = (hash_value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return hash_value


def sanitize_project_name(value: str) -> str:
    cleaned = "".join(
        ch if ch.isascii() and (ch.isalnum() or ch in "-_") else "-" for ch in value
    ).strip("-")
    return cleaned or "root"


def project_id(root: Path) -> str:
    canonical = root.resolve()
    name = sanitize_project_name(canonical.name or "root")
    return f"{name}-{fnv1a64(str(canonical).encode()):016x}"


def state_root() -> Path:
    override = os.environ.get("MIXMOD_STATE_DIR")
    if override:
        return Path(override).expanduser().resolve()
    xdg_state = os.environ.get("XDG_STATE_HOME")
    if xdg_state:
        return Path(xdg_state).expanduser().resolve() / "mixmod"
    home = os.environ.get("HOME")
    if home:
        return Path(home).expanduser().resolve() / ".local" / "state" / "mixmod"
    return Path(tempfile.gettempdir()) / "mixmod"


def project_state(root: Path) -> Path:
    return state_root() / "projects" / project_id(root)


def safe_name(value: str) -> str:
    return value.replace("/", "__")


def repo_key(repo: str) -> str:
    return repo.replace("/", "__")


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def unique_paths(paths: list[str]) -> list[str]:
    seen = set()
    unique = []
    for path in paths:
        if not path or path in seen:
            continue
        seen.add(path)
        unique.append(path)
    return unique


def detected_libstdcxx_dirs() -> list[str]:
    nix_store = Path("/nix/store")
    if not nix_store.exists():
        return []
    matches = []
    for pattern in [
        "*gcc*-lib/lib/libstdc++.so.6",
        "*gfortran*-lib/lib/libstdc++.so.6",
        "*profile/lib/libstdc++.so.6",
    ]:
        matches.extend(nix_store.glob(pattern))
    return unique_paths([str(path.parent) for path in sorted(matches)])


def swebench_eval_env() -> dict[str, str]:
    extra = os.environ.get("MIXMOD_SWEBENCH_EXTRA_LD_LIBRARY_PATH", "")
    paths = [
        *extra.split(os.pathsep),
        "/run/opengl-driver/lib",
        "/run/current-system/sw/lib",
        *detected_libstdcxx_dirs(),
        *os.environ.get("LD_LIBRARY_PATH", "").split(os.pathsep),
    ]
    return {"LD_LIBRARY_PATH": os.pathsep.join(unique_paths(paths))}


def agent_venv_bin(venv: Path) -> Path:
    return venv / ("Scripts" if os.name == "nt" else "bin")


def agent_venv_python(venv: Path) -> Path:
    return agent_venv_bin(venv) / ("python.exe" if os.name == "nt" else "python")


def swebench_agent_env(
    extra: dict[str, str] | None = None, venv: Path | None = None
) -> dict[str, str]:
    """Return environment overrides for local SWE-bench agent commands."""
    env = {
        "PYTHONNOUSERSITE": "1",
        "PIP_DISABLE_PIP_VERSION_CHECK": "1",
        "PIP_NO_INPUT": "1",
        "PYTEST_DISABLE_PLUGIN_AUTOLOAD": "1",
    }
    if venv:
        env["VIRTUAL_ENV"] = str(venv)
        env["PATH"] = str(agent_venv_bin(venv)) + os.pathsep + os.environ.get("PATH", "")
    if extra:
        env.update(extra)
    return env


def run_logged(
    cmd: list[str | Path],
    cwd: Path,
    log: Path,
    env: dict[str, str] | None = None,
    timeout_seconds: int | None = None,
) -> tuple[int, float, bool]:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)

    log.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    with log.open("ab") as handle:
        handle.write(("\n$ " + " ".join(map(str, cmd)) + "\n").encode())
        handle.flush()
        try:
            proc = subprocess.run(
                list(map(str, cmd)),
                cwd=cwd,
                env=merged_env,
                stdout=handle,
                stderr=subprocess.STDOUT,
                check=False,
                timeout=timeout_seconds,
            )
            return proc.returncode, time.monotonic() - started, False
        except subprocess.TimeoutExpired:
            handle.write(f"\nTIMEOUT after {timeout_seconds} seconds\n".encode())
            return 124, time.monotonic() - started, True


def choose_task(root: Path, instance_id: str) -> Path:
    for prefix in TASK_PREFIXES:
        path = root / ".mixmod" / "experiments" / f"{prefix}{instance_id}" / "task.json"
        if path.exists():
            return path

    matches = sorted((root / ".mixmod" / "experiments").glob(f"*{instance_id}/task.json"))
    if not matches:
        raise RuntimeError(f"missing task JSON for {instance_id}")
    return matches[-1]


def parse_task(instance_id: str, task_json: Path) -> InstanceTask:
    task_text = json.dumps(read_json(task_json))
    match = re.search(
        r"Resolve SWE-bench Lite instance .*? for ([^ ]+) at base commit ([0-9a-f]{40})",
        task_text,
    )
    if not match:
        raise RuntimeError(f"could not parse repo/base commit from {task_json}")
    return InstanceTask(
        instance_id=instance_id,
        repo=match.group(1),
        base_commit=match.group(2),
        task_json=task_json,
    )


def find_run_dir(state_dir: Path) -> Path | None:
    candidates = list(state_dir.glob("projects/*/runs/run-*"))
    if not candidates:
        return None
    return max(candidates, key=lambda path: path.stat().st_mtime)


def read_run_metrics(run_dir: Path | None) -> dict[str, Any]:
    if run_dir is None:
        return {}
    metrics_path = run_dir / "metrics.json"
    if not metrics_path.exists():
        return {}
    return read_json(metrics_path)


def selected_instances(args: argparse.Namespace) -> list[str]:
    if args.only:
        selected = args.only
    else:
        selected = SELECTED_INSTANCES
    if args.limit:
        selected = selected[: args.limit]
    return selected


def prepare_worktree(root: Path, run_parent: Path, task: InstanceTask) -> Path:
    worktree = run_parent / "work"
    if worktree.exists():
        shutil.rmtree(worktree)

    cache = root / ".mixmod" / "swebench" / "repo-cache" / repo_key(task.repo)
    if not cache.exists():
        raise RuntimeError(f"missing repo cache for {task.repo}: {cache}")

    setup_log = run_parent / "logs" / "setup.log"
    code, _, _ = run_logged(["git", "clone", "--quiet", cache, worktree], root, setup_log)
    if code != 0:
        raise RuntimeError(f"git clone failed for {task.instance_id}: {code}")

    code, _, _ = run_logged(
        ["git", "-C", worktree, "checkout", "--quiet", task.base_commit],
        root,
        setup_log,
    )
    if code != 0:
        run_logged(["git", "-C", cache, "fetch", "origin", task.base_commit], root, setup_log)
        run_logged(["git", "-C", worktree, "fetch", "origin", task.base_commit], root, setup_log)
        code, _, _ = run_logged(
            ["git", "-C", worktree, "checkout", "--quiet", task.base_commit],
            root,
            setup_log,
        )
        if code != 0:
            raise RuntimeError(f"git checkout failed for {task.instance_id}: {code}")

    shutil.copy2(task.task_json, worktree / "task.json")
    return worktree


def prepare_agent_python_env(worktree: Path, run_parent: Path) -> Path:
    """Create a clean local Python environment for agent-side checks."""
    venv = run_parent / "agent-env" / "venv"
    log = run_parent / "logs" / "agent-env.log"
    if not agent_venv_python(venv).exists():
        code, _, _ = run_logged(
            [sys.executable, "-m", "venv", venv],
            worktree,
            log,
            env=swebench_agent_env(),
            timeout_seconds=15 * 60,
        )
        if code != 0:
            raise RuntimeError(f"agent Python venv creation failed: {code}")

    code, _, _ = run_logged(
        [agent_venv_python(venv), "-m", "pip", "install", "-q", "-e", "."],
        worktree,
        log,
        env=swebench_agent_env(venv=venv),
        timeout_seconds=15 * 60,
    )
    if code != 0:
        raise RuntimeError(f"agent Python editable install failed: {code}")
    return venv


def init_mixmod(
    root: Path, worktree: Path, state_dir: Path, run_parent: Path, agent_venv: Path
) -> None:
    run_logged(
        [root / "target" / "debug" / "mixmod", "init"],
        worktree,
        run_parent / "logs" / "init.log",
        env=swebench_agent_env(
            {"MIXMOD_DEBUG_COMMANDS": "1", "MIXMOD_STATE_DIR": str(state_dir)},
            venv=agent_venv,
        ),
    )


def run_mixmod_exec(
    root: Path,
    worktree: Path,
    run_parent: Path,
    state_dir: Path,
    agent_venv: Path,
    args: argparse.Namespace,
) -> tuple[int, float, bool]:
    return run_logged(
        [
            root / "target" / "debug" / "mixmod",
            "exec",
            "--task",
            "task.json",
            "--supervisor-model",
            args.supervisor_model,
            "--worker-model",
            args.worker_model,
        ],
        worktree,
        run_parent / "logs" / "mixmod-exec.log",
        env=swebench_agent_env({"MIXMOD_STATE_DIR": str(state_dir)}, venv=agent_venv),
        timeout_seconds=args.mixmod_timeout_seconds,
    )


def write_prediction_jsonl(
    instance_id: str,
    model_name: str,
    patch_path: Path,
    output: Path,
) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(
            {
                "instance_id": instance_id,
                "model_name_or_path": model_name,
                "model_patch": patch_path.read_text(),
            }
        )
        + "\n"
    )


def move_eval_summary(root: Path, run_parent: Path, model_name: str, run_id: str) -> Path | None:
    name = f"{model_name}.{run_id}.json"
    src = root / name
    dst = run_parent / "eval" / name
    dst.parent.mkdir(parents=True, exist_ok=True)
    if src.exists():
        shutil.move(str(src), str(dst))
        return dst
    matches = sorted((run_parent / "eval").glob(name))
    return matches[-1] if matches else None


def evaluate_patch(
    root: Path,
    run_parent: Path,
    instance_id: str,
    patch_path: Path,
    args: argparse.Namespace,
) -> dict[str, Any]:
    run_id = f"{args.batch_name}-{safe_name(instance_id)}"
    prediction = run_parent / "eval" / f"{safe_name(instance_id)}.jsonl"
    write_prediction_jsonl(instance_id, args.model_name, patch_path, prediction)

    code, seconds, timed_out = run_logged(
        [
            "swebench-eval",
            "--dataset_name",
            "SWE-bench/SWE-bench_Lite",
            "--split",
            "test",
            "--instance_ids",
            instance_id,
            "--predictions_path",
            prediction,
            "--max_workers",
            "1",
            "--run_id",
            run_id,
            "--report_dir",
            run_parent / "eval",
        ],
        root,
        run_parent / "logs" / "eval.log",
        env=swebench_eval_env(),
        timeout_seconds=args.eval_timeout_seconds,
    )
    summary = move_eval_summary(root, run_parent, args.model_name, run_id)
    resolved = False
    if code == 0 and summary and summary.exists():
        resolved = instance_id in read_json(summary).get("resolved_ids", [])
    return {
        "eval_exit": code,
        "eval_seconds": round(seconds, 3),
        "eval_timed_out": timed_out,
        "official_summary": str(summary) if summary else None,
        "resolved": resolved,
    }


def parse_reuse(value: str) -> tuple[str, Path]:
    try:
        instance_id, path = value.split("=", 1)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("--reuse-run values must be INSTANCE=PATH") from exc
    if not instance_id or not path:
        raise argparse.ArgumentTypeError("--reuse-run values must be INSTANCE=PATH")
    return instance_id, Path(path).expanduser().resolve()


def import_reused_run(root: Path, instance_id: str, run_parent: Path) -> dict[str, Any]:
    task = parse_task(instance_id, choose_task(root, instance_id))
    run_dir = find_run_dir(run_parent / "state")
    metrics = read_run_metrics(run_dir)
    patch = run_dir / "final.patch" if run_dir else None
    summaries = sorted((run_parent / "eval").glob("*.json"))
    summary = summaries[-1] if summaries else None
    resolved = False
    if summary:
        resolved = instance_id in read_json(summary).get("resolved_ids", [])

    record = base_record(task, run_parent, run_dir)
    record.update(
        {
            "mixmod_exit": 0,
            "patch_bytes": patch.stat().st_size if patch and patch.exists() else 0,
            "reused_run": True,
            "resolved": resolved,
            "official_summary": str(summary) if summary else None,
        }
    )
    copy_metric_fields(record, metrics)
    return record


def base_record(task: InstanceTask, run_parent: Path, run_dir: Path | None) -> dict[str, Any]:
    return {
        "base_commit": task.base_commit,
        "instance_id": task.instance_id,
        "repo": task.repo,
        "run_dir": str(run_dir) if run_dir else None,
        "run_parent": str(run_parent),
        "task_json": str(task.task_json),
    }


def copy_metric_fields(record: dict[str, Any], metrics: dict[str, Any]) -> None:
    for key in [
        "codex_calls",
        "final_status",
        "final_verdict",
        "supervisor_cached_input_tokens",
        "supervisor_input_tokens",
        "supervisor_output_tokens",
        "supervisor_total_tokens",
        "gpu_activity_observed",
        "local_inference_verified",
        "opencode_calls",
        "opencode_model",
        "opencode_model_arg",
        "supervision_turn_count",
    ]:
        record[key] = metrics.get(key)


def run_instance(root: Path, batch_dir: Path, task: InstanceTask, args: argparse.Namespace) -> dict[str, Any]:
    run_parent = batch_dir / safe_name(task.instance_id)
    state_dir = run_parent / "state"
    state_dir.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()

    worktree = prepare_worktree(root, run_parent, task)
    agent_venv = prepare_agent_python_env(worktree, run_parent)
    init_mixmod(root, worktree, state_dir, run_parent, agent_venv)
    code, mixmod_seconds, mixmod_timed_out = run_mixmod_exec(
        root, worktree, run_parent, state_dir, agent_venv, args
    )

    run_dir = find_run_dir(state_dir)
    metrics = read_run_metrics(run_dir)
    patch_path = run_dir / "final.patch" if run_dir else None
    patch_bytes = patch_path.stat().st_size if patch_path and patch_path.exists() else 0

    record = base_record(task, run_parent, run_dir)
    record.update(
        {
            "mixmod_exit": code,
            "mixmod_seconds": round(mixmod_seconds, 3),
            "mixmod_timed_out": mixmod_timed_out,
            "patch_bytes": patch_bytes,
            "reused_run": False,
            "agent_python_venv": str(agent_venv),
        }
    )
    copy_metric_fields(record, metrics)

    if args.evaluate and code == 0 and patch_path and patch_bytes > 0:
        record.update(evaluate_patch(root, run_parent, task.instance_id, patch_path, args))
    else:
        record.update({"eval_exit": None, "official_summary": None, "resolved": False})

    record["total_seconds"] = round(time.monotonic() - started, 3)
    return record


def make_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--batch-name", default="")
    parser.add_argument("--pool-name", default=DEFAULT_POOL_NAME)
    parser.add_argument("--model-name", default=DEFAULT_MODEL_NAME)
    parser.add_argument("--supervisor-model", default=DEFAULT_SUPERVISOR_MODEL)
    parser.add_argument("--worker-model", default=DEFAULT_WORKER_MODEL)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--reuse-run", action="append", type=parse_reuse, default=[])
    parser.add_argument("--no-evaluate", dest="evaluate", action="store_false")
    parser.add_argument("--mixmod-timeout-seconds", type=int, default=45 * 60)
    parser.add_argument("--eval-timeout-seconds", type=int, default=45 * 60)
    parser.set_defaults(evaluate=True)
    return parser


def main() -> int:
    args = make_arg_parser().parse_args()
    root = args.root.resolve()
    if not args.batch_name:
        timestamp = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
        args.batch_name = f"{args.pool_name}-{timestamp}"

    batch_dir = project_state(root) / "swebench" / "reruns" / args.batch_name
    batch_dir.mkdir(parents=True, exist_ok=True)
    state_path = batch_dir / "batch-state.json"
    reuse = dict(args.reuse_run)

    state: dict[str, Any] = {
        "batch": str(batch_dir),
        "batch_name": args.batch_name,
        "model_name": args.model_name,
        "records": [],
        "started_at": datetime.now(timezone.utc).isoformat(),
        "supervisor_model": args.supervisor_model,
        "worker_model": args.worker_model,
    }
    write_json(state_path, state)
    print(f"BATCH_START {batch_dir}", flush=True)

    for index, instance_id in enumerate(selected_instances(args), start=1):
        if instance_id in reuse:
            record = import_reused_run(root, instance_id, reuse[instance_id])
            state["records"].append(record)
            write_json(state_path, state)
            print(
                f"BATCH_REUSE {instance_id} resolved={record.get('resolved')} "
                f"input={record.get('supervisor_input_tokens')} "
                f"output={record.get('supervisor_output_tokens')}",
                flush=True,
            )
            continue

        task = parse_task(instance_id, choose_task(root, instance_id))
        print(f"BATCH_ITEM_START {index} {instance_id} repo={task.repo}", flush=True)
        try:
            record = run_instance(root, batch_dir, task, args)
            print(
                f"BATCH_ITEM_DONE {instance_id} resolved={record.get('resolved')} "
                f"status={record.get('final_status')} "
                f"input={record.get('supervisor_input_tokens')} "
                f"output={record.get('supervisor_output_tokens')}",
                flush=True,
            )
        except Exception as exc:  # noqa: BLE001 - benchmark batch should continue.
            record = {
                "error": str(exc),
                "instance_id": instance_id,
                "resolved": False,
            }
            print(f"BATCH_ITEM_ERROR {instance_id} error={exc}", flush=True)
        state["records"].append(record)
        state["updated_at"] = datetime.now(timezone.utc).isoformat()
        write_json(state_path, state)

    state["finished_at"] = datetime.now(timezone.utc).isoformat()
    write_json(state_path, state)
    resolved = sum(1 for record in state["records"] if record.get("resolved"))
    total_input = sum(record.get("supervisor_input_tokens") or 0 for record in state["records"])
    total_output = sum(record.get("supervisor_output_tokens") or 0 for record in state["records"])
    print(
        f"BATCH_DONE records={len(state['records'])} resolved={resolved} "
        f"input={total_input} output={total_output} state={state_path}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
