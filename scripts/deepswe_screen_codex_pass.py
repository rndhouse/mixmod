#!/usr/bin/env python3
"""Screen DeepSWE tasks with Codex/GPT-5.5 through Mixmod app-server."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from scripts.deepswe_artifacts import normalize_pier_job


DEFAULT_MODEL = "openai/gpt-5.5"
DEFAULT_REASONING_EFFORT = "high"
LOCAL_MIXMOD_TARGET = "x86_64-unknown-linux-musl"


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


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def run_logged(
    cmd: list[str | Path],
    cwd: Path,
    log: Path,
    timeout_seconds: int | None,
    env: dict[str, str] | None = None,
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


def ensure_deepswe_repo(path: Path, clone_if_missing: bool) -> Path:
    if path.exists():
        return path.resolve()
    if not clone_if_missing:
        raise RuntimeError(
            f"DeepSWE repo not found at {path}; pass --clone-if-missing to clone it"
        )
    path.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["git", "clone", "https://github.com/datacurve-ai/deep-swe", str(path)],
        check=True,
    )
    return path.resolve()


def reward_value(data: dict[str, Any]) -> float | None:
    for key in ["reward", "score", "pass_rate", "passed_fraction"]:
        value = data.get(key)
        if isinstance(value, bool):
            return 1.0 if value else 0.0
        if isinstance(value, (int, float)):
            return float(value)
    if isinstance(data.get("passed"), bool):
        return 1.0 if data["passed"] else 0.0
    return None


def task_name_from_reward(job_dir: Path, reward_path: Path) -> str:
    trial_dir = reward_path.parent.parent
    result_path = trial_dir / "result.json"
    if result_path.exists():
        try:
            result = read_json(result_path)
        except Exception:  # noqa: BLE001 - fall back to the path-derived name.
            result = {}
        task_id = result.get("task_id")
        if isinstance(task_id, dict):
            task_path = task_id.get("path")
            if isinstance(task_path, str) and task_path:
                return Path(task_path).name
        task_name = result.get("task_name")
        if isinstance(task_name, str) and task_name:
            return task_name.rsplit("/", 1)[-1]
        trial_name = result.get("trial_name")
        if isinstance(trial_name, str) and "__" in trial_name:
            return trial_name.split("__", 1)[0]

    rel = reward_path.relative_to(job_dir)
    for part in rel.parts:
        if part.startswith("trial-") or part in {"logs", "verifier"}:
            continue
        if part.endswith(".json"):
            continue
        return part
    return reward_path.parent.parent.name


def collect_records(job_dir: Path) -> list[dict[str, Any]]:
    records = []
    for reward_path in sorted(job_dir.rglob("reward.json")):
        try:
            reward = read_json(reward_path)
        except Exception as exc:  # noqa: BLE001 - benchmark scans should continue.
            records.append(
                {
                    "task": task_name_from_reward(job_dir, reward_path),
                    "reward_json": str(reward_path),
                    "resolved": False,
                    "error": f"failed to parse reward.json: {exc}",
                }
            )
            continue
        value = reward_value(reward)
        records.append(
            {
                "task": task_name_from_reward(job_dir, reward_path),
                "reward": value,
                "resolved": bool(value is not None and value >= 1.0),
                "reward_json": str(reward_path),
                "ctrf_json": str(reward_path.with_name("ctrf.json")),
                "test_stdout": str(reward_path.with_name("test-stdout.txt")),
            }
        )
    return records


def supervisor_model_arg(args: argparse.Namespace) -> str:
    model = args.model.split("/", 1)[-1]
    if ":" in model:
        return model
    return f"{model}:{args.reasoning_effort}"


def require_codex_auth() -> None:
    explicit = os.environ.get("CODEX_AUTH_JSON_PATH")
    if explicit and Path(explicit).expanduser().is_file():
        return
    if os.environ.get("OPENAI_API_KEY"):
        return
    if (Path.home() / ".codex" / "auth.json").is_file():
        return
    raise RuntimeError(
        "Codex auth is required; set OPENAI_API_KEY, CODEX_AUTH_JSON_PATH, "
        "or provide ~/.codex/auth.json"
    )


def build_pier_command(
    args: argparse.Namespace,
    jobs_dir: Path,
    job_name: str,
    local_mixmod_binary: Path | None,
) -> list[str]:
    cmd = [
        "uvx",
        "--from",
        "datacurve-pier",
        "pier",
        "run",
        "--path",
        str(args.deep_swe / "tasks"),
        "--agent-import-path",
        "scripts.deepswe_codex_app_agent:CodexAppAgent",
        "--model",
        "mixmod/gpt-5.5-app-server",
        "--agent-kwarg",
        f"supervisor_model={supervisor_model_arg(args)}",
        "--agent-kwarg",
        f"mixmod_timeout_sec={args.mixmod_timeout_seconds}",
        "--jobs-dir",
        str(jobs_dir),
        "--job-name",
        job_name,
        "--agent-timeout-multiplier",
        str(args.agent_timeout_multiplier),
        "--n-concurrent",
        "1",
        "--n-tasks",
        str(args.limit),
        "--yes",
    ]
    if local_mixmod_binary is not None:
        cmd.extend(["--agent-kwarg", f"local_mixmod_binary={local_mixmod_binary}"])
    if args.mixmod_install_command:
        cmd.extend(["--agent-kwarg", f"mixmod_install_command={args.mixmod_install_command}"])
    if args.task:
        cmd.extend(["--include-task-name", args.task])
    if args.sample_seed is not None:
        cmd.extend(["--sample-seed", str(args.sample_seed)])
    if args.no_delete:
        cmd.append("--no-delete")
    return cmd


def build_local_mixmod_binary(root: Path, log: Path) -> Path:
    code, _, timed_out = run_logged(
        ["cargo", "build", "--target", LOCAL_MIXMOD_TARGET, "--bin", "mixmod"],
        root,
        log,
        timeout_seconds=None,
    )
    if code != 0 or timed_out:
        raise RuntimeError(f"local Mixmod binary build failed; see {log}")
    binary = root / "target" / LOCAL_MIXMOD_TARGET / "debug" / "mixmod"
    if not binary.is_file():
        raise RuntimeError(f"local Mixmod binary was not produced: {binary}")
    return binary.resolve()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--deep-swe", type=Path, default=Path("../deep-swe"))
    parser.add_argument("--clone-if-missing", action="store_true")
    parser.add_argument("--task", help="Optional DeepSWE task name or glob.")
    parser.add_argument("--limit", type=int, default=1)
    parser.add_argument("--sample-seed", type=int)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--reasoning-effort", default=DEFAULT_REASONING_EFFORT)
    parser.add_argument("--mixmod-install-command")
    parser.add_argument("--local-mixmod-binary", type=Path)
    parser.add_argument("--no-local-mixmod-binary", action="store_true")
    parser.add_argument("--mixmod-timeout-seconds", type=int, default=3 * 60 * 60)
    parser.add_argument("--agent-timeout-multiplier", type=float, default=3.0)
    parser.add_argument("--job-name", default="")
    parser.add_argument("--timeout-seconds", type=int, default=3 * 60 * 60)
    parser.add_argument("--no-delete", action="store_true")
    args = parser.parse_args()

    root = args.root.resolve()
    require_codex_auth()
    args.deep_swe = ensure_deepswe_repo(args.deep_swe.expanduser(), args.clone_if_missing)
    job_name = args.job_name or datetime.now(timezone.utc).strftime(
        "deepswe-codex-screen-%Y%m%d%H%M%S"
    )
    pool = project_state(root) / "deepswe" / "codex-screen" / job_name
    jobs_dir = pool / "pier-jobs"
    log = pool / "logs" / "pier-run.log"
    local_mixmod_binary = None
    if not args.no_local_mixmod_binary:
        local_mixmod_binary = (
            args.local_mixmod_binary.expanduser().resolve()
            if args.local_mixmod_binary
            else build_local_mixmod_binary(root, pool / "logs" / "local-mixmod-build.log")
        )
        if not local_mixmod_binary.is_file():
            raise RuntimeError(f"local Mixmod binary not found: {local_mixmod_binary}")
    cmd = build_pier_command(args, jobs_dir, job_name, local_mixmod_binary)

    state: dict[str, Any] = {
        "kind": "deepswe-codex-app-screen",
        "job_name": job_name,
        "deep_swe": str(args.deep_swe),
        "model": args.model,
        "reasoning_effort": args.reasoning_effort,
        "supervisor_model": supervisor_model_arg(args),
        "limit": args.limit,
        "task": args.task,
        "agent_timeout_multiplier": args.agent_timeout_multiplier,
        "mixmod_timeout_seconds": args.mixmod_timeout_seconds,
        "local_mixmod_binary": str(local_mixmod_binary) if local_mixmod_binary else None,
        "command": [str(part) for part in cmd],
        "started_at": datetime.now(timezone.utc).isoformat(),
    }
    write_json(pool / "screen-state.json", state)
    print(f"DEEPSWE_CODEX_SCREEN_START job={job_name} limit={args.limit}", flush=True)
    pythonpath = str(root)
    if os.environ.get("PYTHONPATH"):
        pythonpath = f"{pythonpath}{os.pathsep}{os.environ['PYTHONPATH']}"
    code, seconds, timed_out = run_logged(
        cmd,
        root,
        log,
        args.timeout_seconds,
        env={"PYTHONPATH": pythonpath},
    )

    job_dir = jobs_dir / job_name
    records = collect_records(job_dir) if job_dir.exists() else []
    artifact_bundles = normalize_pier_job(pool, job_dir, records) if job_dir.exists() else []
    state.update(
        {
            "exit": code,
            "seconds": round(seconds, 3),
            "timed_out": timed_out,
            "finished_at": datetime.now(timezone.utc).isoformat(),
            "job_dir": str(job_dir),
            "log": str(log),
            "records": records,
            "artifact_bundles": artifact_bundles,
            "artifact_bundles_manifest": str(pool / "artifact-bundles.json"),
            "tasks_dir": str(pool / "tasks"),
            "resolved": [record for record in records if record.get("resolved")],
        }
    )
    write_json(pool / "screen-state.json", state)
    print(
        "DEEPSWE_CODEX_SCREEN_DONE "
        f"exit={code} records={len(records)} resolved={len(state['resolved'])} "
        f"bundles={len(artifact_bundles)} state={pool / 'screen-state.json'} "
        f"tasks={pool / 'tasks'}",
        flush=True,
    )
    return code


if __name__ == "__main__":
    sys.exit(main())
