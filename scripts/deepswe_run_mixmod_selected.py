#!/usr/bin/env python3
"""Run Mixmod on a Codex-resolved DeepSWE screen pool through Pier."""

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
from urllib.parse import urlparse


DEFAULT_SUPERVISOR_MODEL = "gpt-5.5:high"
DEFAULT_WORKER_MODEL = "llama.cpp/qwen/qwen3.6-27b"
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


def collect_records(job_dir: Path) -> list[dict[str, Any]]:
    records = []
    for reward_path in sorted(job_dir.rglob("reward.json")):
        reward = read_json(reward_path)
        value = reward.get("reward")
        records.append(
            {
                "reward": value,
                "resolved": bool(value == 1 or value is True),
                "reward_json": str(reward_path),
                "ctrf_json": str(reward_path.with_name("ctrf.json")),
                "test_stdout": str(reward_path.with_name("test-stdout.txt")),
            }
        )
    return records


def selected_tasks(screen_state: dict[str, Any], limit: int) -> list[str]:
    tasks = [
        record["task"]
        for record in screen_state.get("resolved", [])
        if record.get("task")
    ]
    return tasks[:limit] if limit else tasks


def add_csv_env_value(agent_env: list[str], key: str, values: list[str]) -> None:
    unique_values = []
    for value in values:
        if value and value not in unique_values:
            unique_values.append(value)
    for index, item in enumerate(agent_env):
        if not item.startswith(f"{key}="):
            continue
        existing = item.split("=", 1)[1]
        parts = [part.strip() for part in existing.split(",") if part.strip()]
        for value in unique_values:
            if value and value not in parts:
                parts.append(value)
        agent_env[index] = f"{key}={','.join(parts)}"
        return
    agent_env.append(f"{key}={','.join(unique_values)}")


def worker_no_proxy_values(worker_base_url: str | None) -> list[str]:
    values = ["localhost", "127.0.0.1"]
    if worker_base_url:
        host = urlparse(worker_base_url).hostname
        if host:
            values.append(host)
    return values


def effective_agent_env(args: argparse.Namespace) -> list[str]:
    agent_env = list(args.agent_env)
    agent_env_keys = {value.split("=", 1)[0] for value in agent_env if "=" in value}
    if args.worker_base_url and "NODE_OPTIONS" not in agent_env_keys:
        agent_env.append("NODE_OPTIONS=--use-env-proxy")
    if args.worker_base_url:
        no_proxy_values = worker_no_proxy_values(args.worker_base_url)
        add_csv_env_value(agent_env, "NO_PROXY", no_proxy_values)
        add_csv_env_value(agent_env, "no_proxy", no_proxy_values)
    return agent_env


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
        "scripts.deepswe_mixmod_agent:MixmodAgent",
        "--model",
        "mixmod/gpt-5.5-qwen",
        "--jobs-dir",
        str(jobs_dir),
        "--job-name",
        job_name,
    ]
    if args.host_network:
        cmd.extend(
            [
                "--environment-import-path",
                "scripts.pier_host_network_env:HostNetworkDockerEnvironment",
            ]
        )
    cmd.extend(
        [
        "--n-concurrent",
        "1",
        "--yes",
        "--agent-kwarg",
        f"supervisor_model={args.supervisor_model}",
        "--agent-kwarg",
        f"worker_model={args.worker_model}",
        "--agent-kwarg",
        "worker_backend=opencode",
        "--agent-kwarg",
        f"supervisor_init={args.supervisor_init}",
        "--agent-kwarg",
        f"stop_after_first_worker={str(args.stop_after_first_worker).lower()}",
        "--agent-kwarg",
        f"require_local={str(args.require_local).lower()}",
        "--agent-kwarg",
        f"mixmod_timeout_sec={args.mixmod_timeout_seconds}",
        ]
    )
    if local_mixmod_binary is not None:
        cmd.extend(["--agent-kwarg", f"local_mixmod_binary={local_mixmod_binary}"])
    if args.mixmod_install_command:
        cmd.extend(["--agent-kwarg", f"mixmod_install_command={args.mixmod_install_command}"])
    if args.worker_base_url:
        cmd.extend(["--agent-kwarg", f"worker_base_url={args.worker_base_url}"])
    for value in effective_agent_env(args):
        cmd.extend(["--agent-env", value])
    for task in args.tasks:
        cmd.extend(["--include-task-name", task])
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
    parser.add_argument("--screen-state", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=1)
    parser.add_argument("--supervisor-model", default=DEFAULT_SUPERVISOR_MODEL)
    parser.add_argument("--worker-model", default=DEFAULT_WORKER_MODEL)
    parser.add_argument(
        "--supervisor-init",
        choices=["compact", "investigate"],
        default="compact",
    )
    parser.add_argument("--stop-after-first-worker", action="store_true")
    parser.add_argument("--no-require-local", dest="require_local", action="store_false")
    parser.add_argument("--worker-base-url", "--ollama-base-url", dest="worker_base_url")
    parser.add_argument("--mixmod-install-command")
    parser.add_argument("--local-mixmod-binary", type=Path)
    parser.add_argument("--no-local-mixmod-binary", action="store_true")
    parser.add_argument(
        "--agent-env",
        action="append",
        default=[],
        help="Environment variable to pass to the Pier agent as KEY=VALUE.",
    )
    parser.add_argument("--mixmod-timeout-seconds", type=int, default=3 * 60 * 60)
    parser.add_argument("--timeout-seconds", type=int, default=4 * 60 * 60)
    parser.add_argument("--job-name", default="")
    parser.add_argument("--no-delete", action="store_true")
    parser.add_argument("--host-network", dest="host_network", action="store_true")
    parser.add_argument("--no-host-network", dest="host_network", action="store_false")
    parser.set_defaults(require_local=True, host_network=None)
    args = parser.parse_args()

    root = args.root.resolve()
    args.deep_swe = args.deep_swe.expanduser().resolve()
    if args.host_network is None:
        args.host_network = bool(args.worker_base_url)
    screen_state = read_json(args.screen_state)
    args.tasks = selected_tasks(screen_state, args.limit)
    if not args.tasks:
        raise RuntimeError(f"no resolved tasks found in {args.screen_state}")

    job_name = args.job_name or datetime.now(timezone.utc).strftime(
        "deepswe-mixmod-%Y%m%d%H%M%S"
    )
    pool = project_state(root) / "deepswe" / "mixmod" / job_name
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
    state = {
        "kind": "deepswe-mixmod",
        "job_name": job_name,
        "deep_swe": str(args.deep_swe),
        "screen_state": str(args.screen_state),
        "tasks": args.tasks,
        "supervisor_model": args.supervisor_model,
        "worker_model": args.worker_model,
        "worker_backend": "opencode",
        "supervisor_init": args.supervisor_init,
        "stop_after_first_worker": args.stop_after_first_worker,
        "require_local": args.require_local,
        "host_network": args.host_network,
        "agent_env": args.agent_env,
        "effective_agent_env": effective_agent_env(args),
        "local_mixmod_binary": str(local_mixmod_binary) if local_mixmod_binary else None,
        "command": [str(part) for part in cmd],
        "started_at": datetime.now(timezone.utc).isoformat(),
    }
    write_json(pool / "mixmod-state.json", state)
    print(f"DEEPSWE_MIXMOD_START job={job_name} tasks={len(args.tasks)}", flush=True)
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
    state.update(
        {
            "exit": code,
            "seconds": round(seconds, 3),
            "timed_out": timed_out,
            "finished_at": datetime.now(timezone.utc).isoformat(),
            "job_dir": str(job_dir),
            "log": str(log),
            "records": records,
            "resolved": [record for record in records if record.get("resolved")],
        }
    )
    write_json(pool / "mixmod-state.json", state)
    print(
        "DEEPSWE_MIXMOD_DONE "
        f"exit={code} records={len(records)} resolved={len(state['resolved'])} "
        f"state={pool / 'mixmod-state.json'}",
        flush=True,
    )
    return code


if __name__ == "__main__":
    sys.exit(main())
