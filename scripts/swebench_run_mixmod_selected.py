#!/usr/bin/env python3
"""Run Mixmod current-default on the selected Codex-pass expansion pool."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


def fnv1a64(value: bytes) -> int:
    hash_value = 0xCBF29CE484222325
    for byte in value:
        hash_value ^= byte
        hash_value = (hash_value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return hash_value


def sanitize_project_name(value: str) -> str:
    cleaned = "".join(ch if ch.isascii() and (ch.isalnum() or ch in "-_") else "-" for ch in value).strip("-")
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


def display_path(root: Path, path: Path) -> str:
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def repo_key(repo: str) -> str:
    return repo.replace("/", "__")


def safe_instance(instance_id: str) -> str:
    return instance_id.replace("/", "__")


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


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


def run(cmd: list[str], cwd: Path, env: dict[str, str] | None = None, log: Path | None = None) -> int:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    if log:
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("ab") as fh:
            fh.write(("\n$ " + " ".join(cmd) + "\n").encode())
            proc = subprocess.run(
                cmd,
                cwd=cwd,
                env=merged_env,
                stdout=fh,
                stderr=subprocess.STDOUT,
                check=False,
            )
        return proc.returncode
    return subprocess.run(cmd, cwd=cwd, env=merged_env, check=False).returncode


def ensure_default_worktree(root: Path, item: dict[str, Any]) -> None:
    state = project_state(root)
    exp_dir = state / "experiments" / item["experiment"]
    work_dir = exp_dir / "work" / "default"
    if work_dir.exists():
        return
    cache = state / "swebench" / "repo-cache" / repo_key(item["repo"])
    if not cache.exists():
        raise RuntimeError(f"missing repo cache for {item['repo']}: {cache}")
    work_dir.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "clone", str(cache), str(work_dir)], cwd=root, check=True)
    checkout = subprocess.run(
        ["git", "-C", str(work_dir), "checkout", item["base_commit"]],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
    )
    if checkout.returncode != 0:
        subprocess.run(["git", "-C", str(cache), "fetch", "origin", item["base_commit"]], cwd=root, check=True)
        subprocess.run(["git", "-C", str(work_dir), "fetch", "origin", item["base_commit"]], cwd=root, check=True)
        subprocess.run(["git", "-C", str(work_dir), "checkout", item["base_commit"]], cwd=root, check=True)


def write_prediction_jsonl(instance_id: str, patch_path: Path, output: Path) -> None:
    pred = {
        "instance_id": instance_id,
        "model_name_or_path": "mixmod-current-default-v1-expansion",
        "model_patch": patch_path.read_text(),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(pred) + "\n")


def move_eval_summary(root: Path, run_id: str) -> Path:
    src = root / f"mixmod-current-default-v1-expansion.{run_id}.json"
    dst = (
        project_state(root)
        / "swebench"
        / "current-default-v1-expansion"
        / "eval"
        / "official-summary"
        / src.name
    )
    dst.parent.mkdir(parents=True, exist_ok=True)
    if src.exists():
        shutil.move(str(src), str(dst))
    return dst


def evaluate(root: Path, item: dict[str, Any], prediction: Path, run_id: str, log_dir: Path) -> tuple[bool, Path | None]:
    log = log_dir / f"eval-mixmod-{item['instance_id']}.txt"
    code = run(
        [
            "swebench-eval",
            "--dataset_name",
            "SWE-bench/SWE-bench_Lite",
            "--split",
            "test",
            "--instance_ids",
            item["instance_id"],
            "--predictions_path",
            str(prediction),
            "--max_workers",
            "1",
            "--run_id",
            run_id,
            "--report_dir",
            str(project_state(root) / "swebench" / "current-default-v1-expansion" / "eval"),
        ],
        cwd=root,
        env=swebench_eval_env(),
        log=log,
    )
    summary = move_eval_summary(root, run_id)
    if code != 0 or not summary.exists():
        return False, summary if summary.exists() else None
    data = read_json(summary)
    return item["instance_id"] in data.get("resolved_ids", []), summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--limit", type=int, default=0, help="Optional max selected instances to run.")
    args = parser.parse_args()

    root = args.root.resolve()
    state = project_state(root)
    pool = state / "swebench" / "current-default-v1-expansion"
    screen_state = read_json(pool / "screen-state.json")
    selected = screen_state["resolved"]
    if args.limit:
        selected = selected[: args.limit]

    state_path = pool / "mixmod-state.json"
    if state_path.exists():
        state = read_json(state_path)
    else:
        state = {"completed": []}
    completed = {item["instance_id"] for item in state.get("completed", [])}
    log_dir = pool / "logs"

    print(f"mixmod_selected={len(selected)} completed={len(completed)}", flush=True)
    for item in selected:
        instance_id = item["instance_id"]
        if instance_id in completed:
            continue
        exp_dir = state / "experiments" / item["experiment"]
        default_dir = exp_dir / "default"
        metrics_path = default_dir / "metrics.json"
        patch_path = default_dir / "final.patch"

        print(f"MIXMOD_START {instance_id} repo={item['repo']}", flush=True)
        ensure_default_worktree(root, item)
        code = run(
            [
                str(root / "target" / "debug" / "mixmod"),
                "experiment",
                "run-default",
                item["experiment"],
                "--require-local",
            ],
            cwd=root,
            log=log_dir / f"mixmod-{instance_id}.txt",
        )

        patch_bytes = patch_path.stat().st_size if patch_path.exists() else 0
        metrics = read_json(metrics_path) if metrics_path.exists() else {}
        resolved = False
        summary_path: Path | None = None
        if code == 0 and patch_bytes > 0:
            run_id = f"current-default-v1-mixmod-{safe_instance(instance_id)}"
            prediction = pool / "eval" / f"mixmod-{instance_id}.jsonl"
            write_prediction_jsonl(instance_id, patch_path, prediction)
            print(f"MIXMOD_EVAL_START {instance_id} patch_bytes={patch_bytes}", flush=True)
            resolved, summary_path = evaluate(root, item, prediction, run_id, log_dir)
        else:
            print(f"MIXMOD_EVAL_SKIP {instance_id} exit={code} patch_bytes={patch_bytes}", flush=True)

        record = {
            "instance_id": instance_id,
            "repo": item["repo"],
            "base_commit": item["base_commit"],
            "experiment": item["experiment"],
            "mixmod_exit": code,
            "patch_bytes": patch_bytes,
            "supervisor_input_tokens": metrics.get("supervisor_input_tokens"),
            "supervisor_output_tokens": metrics.get("supervisor_output_tokens"),
            "supervisor_total_tokens": metrics.get("supervisor_total_tokens"),
            "opencode_calls": metrics.get("opencode_calls"),
            "codex_calls": metrics.get("codex_calls"),
            "opencode_model_arg": metrics.get("opencode_model_arg"),
            "local_inference_verified": metrics.get("local_inference_verified"),
            "gpu_activity_observed": metrics.get("gpu_activity_observed"),
            "final_status": metrics.get("final_status"),
            "final_verdict": metrics.get("final_verdict"),
            "resolved": resolved,
            "official_summary": display_path(root, summary_path) if summary_path else None,
        }
        state.setdefault("completed", []).append(record)
        completed.add(instance_id)
        write_json(state_path, state)
        print(f"MIXMOD_DONE {instance_id} resolved={resolved}", flush=True)

    print(f"MIXMOD_BATCH_DONE completed={len(completed)}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
