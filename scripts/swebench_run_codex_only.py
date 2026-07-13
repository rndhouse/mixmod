#!/usr/bin/env python3
"""Run a selected SWE-bench Lite task through direct `codex exec`."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.swebench_run_exec_batch import (  # noqa: E402
    InstanceTask,
    choose_task,
    evaluate_patch,
    parse_task,
    prepare_agent_python_env,
    prepare_worktree,
    project_state,
    read_json,
    safe_name,
    selected_instances,
    swebench_agent_env,
    write_json,
)


DEFAULT_MODEL = "gpt-5.5"
DEFAULT_REASONING_EFFORT = "high"
DEFAULT_MODEL_NAME = "codex-direct-gpt-5.5-high"


def split_model(value: str, effort: str) -> tuple[str, str]:
    if ":" in value:
        model, parsed_effort = value.rsplit(":", 1)
        return model, parsed_effort
    return value, effort


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


def prepare_codex_home(run_parent: Path) -> Path:
    codex_home = run_parent / "codex-home"
    if codex_home.exists():
        shutil.rmtree(codex_home)
    codex_home.mkdir(parents=True)

    explicit = os.environ.get("CODEX_AUTH_JSON_PATH")
    auth_source = (
        Path(explicit).expanduser()
        if explicit
        else Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")) / "auth.json"
    )
    if auth_source.is_file():
        shutil.copy2(auth_source, codex_home / "auth.json")

    return codex_home


def build_prompt(task: InstanceTask) -> str:
    data = read_json(task.task_json)
    return "\n".join(
        [
            "Solve this SWE-bench Lite task in the current repository.",
            "",
            "Modify files as needed to address the issue. Run relevant checks if useful.",
            "Do not commit. Leave the final solution as an uncommitted git diff.",
            "",
            "Task JSON:",
            "```json",
            json.dumps(data, indent=2, sort_keys=True),
            "```",
            "",
        ]
    )


def run_codex_exec(
    worktree: Path,
    run_parent: Path,
    codex_home: Path,
    agent_venv: Path,
    args: argparse.Namespace,
) -> tuple[int, float, bool]:
    model, effort = split_model(args.model, args.reasoning_effort)
    agent_dir = run_parent / "codex"
    prompt_path = agent_dir / "codex-prompt.md"
    stdout_path = agent_dir / "codex.exec.stdout.jsonl"
    stderr_path = agent_dir / "codex.exec.stderr.txt"
    last_message_path = agent_dir / "codex-last-message.md"
    agent_dir.mkdir(parents=True, exist_ok=True)

    cmd = [
        "codex",
        "--ask-for-approval",
        "never",
        "exec",
        "--json",
        "--model",
        model,
        "--sandbox",
        "danger-full-access",
        "--cd",
        str(worktree),
        "--config",
        f'model_reasoning_effort="{effort}"',
        "--config",
        "shell_environment_policy.inherit=all",
        "--output-last-message",
        str(last_message_path),
        "-",
    ]
    write_json(
        run_parent / "codex-command.json",
        {
            "cmd": cmd,
            "codex_home": str(codex_home),
            "cwd": str(worktree),
            "model": model,
            "reasoning_effort": effort,
        },
    )

    started = time.monotonic()
    env = os.environ.copy()
    env.update(swebench_agent_env({"CODEX_HOME": str(codex_home)}, venv=agent_venv))
    if os.environ.get("OPENAI_API_KEY"):
        env["OPENAI_API_KEY"] = os.environ["OPENAI_API_KEY"]

    with prompt_path.open("rb") as prompt, stdout_path.open("wb") as stdout, stderr_path.open(
        "wb"
    ) as stderr:
        try:
            proc = subprocess.run(
                cmd,
                cwd=worktree,
                env=env,
                stdin=prompt,
                stdout=stdout,
                stderr=stderr,
                check=False,
                timeout=args.codex_timeout_seconds,
            )
            return proc.returncode, time.monotonic() - started, False
        except subprocess.TimeoutExpired:
            stderr.write(f"\nTIMEOUT after {args.codex_timeout_seconds} seconds\n".encode())
            return 124, time.monotonic() - started, True


def copy_codex_rollouts(run_parent: Path, codex_home: Path) -> Path:
    destination = run_parent / "codex" / "codex-rollouts"
    destination.mkdir(parents=True, exist_ok=True)
    source = codex_home / "sessions"
    if source.exists():
        shutil.copytree(source, destination, dirs_exist_ok=True)
    return destination


def zero_usage() -> dict[str, int]:
    return {
        "input_tokens": 0,
        "cached_input_tokens": 0,
        "output_tokens": 0,
        "reasoning_tokens": 0,
        "total_tokens": 0,
    }


def snake_usage(value: dict[str, Any]) -> dict[str, int]:
    return {
        "input_tokens": int(value.get("input_tokens") or 0),
        "cached_input_tokens": int(value.get("cached_input_tokens") or 0),
        "output_tokens": int(value.get("output_tokens") or 0),
        "reasoning_tokens": int(
            value.get("reasoning_output_tokens") or value.get("reasoning_tokens") or 0
        ),
        "total_tokens": int(value.get("total_tokens") or 0),
    }


def camel_usage(value: dict[str, Any]) -> dict[str, int]:
    return {
        "input_tokens": int(value.get("inputTokens") or 0),
        "cached_input_tokens": int(value.get("cachedInputTokens") or 0),
        "output_tokens": int(value.get("outputTokens") or 0),
        "reasoning_tokens": int(
            value.get("reasoningOutputTokens") or value.get("reasoningTokens") or 0
        ),
        "total_tokens": int(value.get("totalTokens") or 0),
    }


def usage_from_value(value: dict[str, Any]) -> dict[str, int] | None:
    payload = value.get("payload")
    if value.get("type") == "event_msg" and isinstance(payload, dict):
        if payload.get("type") == "token_count":
            info = payload.get("info") or {}
            total = info.get("total_token_usage")
            if isinstance(total, dict):
                return snake_usage(total)
    if value.get("type") == "token_count":
        info = value.get("info") or {}
        total = info.get("total_token_usage")
        if isinstance(total, dict):
            return snake_usage(total)
    if value.get("method") == "thread/tokenUsage/updated":
        params = value.get("params") or {}
        token_usage = params.get("tokenUsage") or {}
        total = token_usage.get("total")
        last = token_usage.get("last")
        if isinstance(total, dict):
            return camel_usage(total)
        if isinstance(last, dict):
            return camel_usage(last)
    return None


def usage_from_jsonl(path: Path) -> dict[str, int]:
    usage = zero_usage()
    try:
        lines = path.read_text(errors="replace").splitlines()
    except OSError:
        return usage
    for line in lines:
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        next_usage = usage_from_value(value)
        if next_usage is not None:
            usage = next_usage
    return usage


def collect_usage(run_parent: Path, rollouts: Path) -> tuple[dict[str, int], str, int]:
    usage = zero_usage()
    source = "none"
    rollout_paths = sorted(rollouts.rglob("rollout-*.jsonl"))
    for rollout in rollout_paths:
        next_usage = usage_from_jsonl(rollout)
        if next_usage["total_tokens"]:
            for key, value in next_usage.items():
                usage[key] += value
            source = "codex_rollout_total_token_usage"

    if not usage["total_tokens"]:
        stdout_path = run_parent / "codex" / "codex.exec.stdout.jsonl"
        next_usage = usage_from_jsonl(stdout_path)
        if next_usage["total_tokens"]:
            usage = next_usage
            source = "codex_stdout_total_token_usage"

    return usage, source, len(rollout_paths)


def write_final_patch(worktree: Path, output: Path) -> int:
    output.parent.mkdir(parents=True, exist_ok=True)
    proc = subprocess.run(
        ["git", "diff", "--binary", "HEAD"],
        cwd=worktree,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    output.write_bytes(proc.stdout)
    return output.stat().st_size


def run_instance(
    root: Path,
    batch_dir: Path,
    task: InstanceTask,
    args: argparse.Namespace,
) -> dict[str, Any]:
    run_parent = batch_dir / safe_name(task.instance_id)
    run_parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()

    worktree = prepare_worktree(root, run_parent, task)
    agent_venv = prepare_agent_python_env(worktree, run_parent)
    codex_home = prepare_codex_home(run_parent)
    prompt = build_prompt(task)
    (run_parent / "codex" / "codex-prompt.md").parent.mkdir(parents=True, exist_ok=True)
    (run_parent / "codex" / "codex-prompt.md").write_text(prompt)

    code, codex_seconds, codex_timed_out = run_codex_exec(
        worktree, run_parent, codex_home, agent_venv, args
    )
    rollouts = copy_codex_rollouts(run_parent, codex_home)
    usage, source, rollout_count = collect_usage(run_parent, rollouts)
    patch_path = run_parent / "final.patch"
    patch_bytes = write_final_patch(worktree, patch_path)
    model, effort = split_model(args.model, args.reasoning_effort)

    metrics = {
        "kind": "swebench-codex-only",
        "runner_mode": "codex-direct",
        "codex_exit_status": code,
        "codex_model": model,
        "codex_reasoning_effort": effort,
        "codex_input_tokens": usage["input_tokens"],
        "codex_cached_input_tokens": usage["cached_input_tokens"],
        "codex_output_tokens": usage["output_tokens"],
        "codex_reasoning_tokens": usage["reasoning_tokens"],
        "codex_total_tokens": usage["total_tokens"],
        "codex_token_usage_source": source,
        "codex_rollout_count": rollout_count,
        "codex_seconds": round(codex_seconds, 3),
        "codex_timed_out": codex_timed_out,
        "agent_python_venv": str(agent_venv),
        "final_status": "success" if code == 0 else "needs_review",
        "patch_bytes": patch_bytes,
        "stdout_bytes": (run_parent / "codex" / "codex.exec.stdout.jsonl").stat().st_size,
        "stderr_bytes": (run_parent / "codex" / "codex.exec.stderr.txt").stat().st_size,
    }
    write_json(run_parent / "codex" / "codex-metrics.json", metrics)

    record: dict[str, Any] = {
        "base_commit": task.base_commit,
        "codex_exit": code,
        "codex_seconds": round(codex_seconds, 3),
        "codex_timed_out": codex_timed_out,
        "instance_id": task.instance_id,
        "patch_bytes": patch_bytes,
        "repo": task.repo,
        "run_parent": str(run_parent),
        "task_json": str(task.task_json),
        **metrics,
    }
    if args.evaluate and code == 0 and patch_bytes > 0:
        record.update(evaluate_patch(root, run_parent, task.instance_id, patch_path, args))
    else:
        record.update({"eval_exit": None, "official_summary": None, "resolved": False})

    record["total_seconds"] = round(time.monotonic() - started, 3)
    return record


def make_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--batch-name", default="")
    parser.add_argument("--model-name", default=DEFAULT_MODEL_NAME)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--reasoning-effort", default=DEFAULT_REASONING_EFFORT)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--no-evaluate", dest="evaluate", action="store_false")
    parser.add_argument("--codex-timeout-seconds", type=int, default=45 * 60)
    parser.add_argument("--eval-timeout-seconds", type=int, default=45 * 60)
    parser.set_defaults(evaluate=True)
    return parser


def main() -> int:
    args = make_arg_parser().parse_args()
    root = args.root.resolve()
    require_codex_auth()
    if not args.batch_name:
        timestamp = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
        args.batch_name = f"swebench-codex-direct-{timestamp}"

    batch_dir = project_state(root) / "swebench" / "codex-only" / args.batch_name
    batch_dir.mkdir(parents=True, exist_ok=True)
    state_path = batch_dir / "codex-only-state.json"
    state: dict[str, Any] = {
        "batch": str(batch_dir),
        "batch_name": args.batch_name,
        "kind": "swebench-codex-only",
        "model": args.model,
        "model_name": args.model_name,
        "reasoning_effort": args.reasoning_effort,
        "records": [],
        "started_at": datetime.now(timezone.utc).isoformat(),
    }
    write_json(state_path, state)
    print(f"CODEX_ONLY_START {batch_dir}", flush=True)

    for index, instance_id in enumerate(selected_instances(args), start=1):
        task = parse_task(instance_id, choose_task(root, instance_id))
        print(f"CODEX_ONLY_ITEM_START {index} {instance_id} repo={task.repo}", flush=True)
        try:
            record = run_instance(root, batch_dir, task, args)
            print(
                f"CODEX_ONLY_ITEM_DONE {instance_id} resolved={record.get('resolved')} "
                f"exit={record.get('codex_exit')} input={record.get('codex_input_tokens')} "
                f"output={record.get('codex_output_tokens')}",
                flush=True,
            )
        except Exception as exc:  # noqa: BLE001 - benchmark batch should continue.
            record = {
                "error": str(exc),
                "instance_id": instance_id,
                "resolved": False,
            }
            print(f"CODEX_ONLY_ITEM_ERROR {instance_id} error={exc}", flush=True)
        state["records"].append(record)
        state["updated_at"] = datetime.now(timezone.utc).isoformat()
        write_json(state_path, state)

    state["finished_at"] = datetime.now(timezone.utc).isoformat()
    write_json(state_path, state)
    resolved = sum(1 for record in state["records"] if record.get("resolved"))
    total_input = sum(record.get("codex_input_tokens") or 0 for record in state["records"])
    total_output = sum(record.get("codex_output_tokens") or 0 for record in state["records"])
    print(
        f"CODEX_ONLY_DONE records={len(state['records'])} resolved={resolved} "
        f"input={total_input} output={total_output} state={state_path}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
