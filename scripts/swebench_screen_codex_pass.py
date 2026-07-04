#!/usr/bin/env python3
"""Screen SWE-bench Lite candidates for Codex-only resolved instances.

This is experiment harness glue, not model-facing logic. It writes generated
artifacts under Mixmod's central state dir and invokes the Mixmod CLI for
Codex-only baselines.
Gold patches are used only to order candidate screening; they are not written
to task prompts or agent worktrees.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from datasets import load_dataset


CACHED_REPOS = {
    "django/django",
    "pytest-dev/pytest",
    "sympy/sympy",
    "scikit-learn/scikit-learn",
    "psf/requests",
    "pallets/flask",
}

EXISTING_SELECTED = {
    "pytest-dev__pytest-11143",
    "scikit-learn__scikit-learn-13439",
    "sympy__sympy-20212",
}


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


@dataclass(frozen=True)
class Candidate:
    instance_id: str
    repo: str
    base_commit: str
    version: str
    problem_statement: str
    fail_to_pass: str
    pass_to_pass: str
    patch_changed_lines: int
    patch_files: int
    test_patch_changed_lines: int


def run(cmd: list[str], cwd: Path, env: dict[str, str] | None = None, log: Path | None = None) -> int:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    if log:
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("wb") as fh:
            proc = subprocess.run(
                cmd,
                cwd=cwd,
                env=merged_env,
                stdout=fh,
                stderr=subprocess.STDOUT,
                check=False,
            )
        return proc.returncode
    proc = subprocess.run(cmd, cwd=cwd, env=merged_env, check=False)
    return proc.returncode


def capture(cmd: list[str], cwd: Path) -> str:
    proc = subprocess.run(cmd, cwd=cwd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if proc.returncode != 0:
        raise RuntimeError(f"command failed ({proc.returncode}): {' '.join(cmd)}\n{proc.stderr}")
    return proc.stdout


def patch_stats(patch: str) -> tuple[int, int]:
    files = 0
    changed = 0
    for line in patch.splitlines():
        if line.startswith("diff --git "):
            files += 1
        elif line.startswith(("---", "+++", "@@")):
            continue
        elif line.startswith(("+", "-")):
            changed += 1
    return files, changed


def safe_instance(instance_id: str) -> str:
    return instance_id.replace("/", "__")


def repo_key(repo: str) -> str:
    return repo.replace("/", "__")


def make_task(candidate: Candidate) -> dict[str, Any]:
    return {
        "title": f"SWE-bench Lite {candidate.instance_id}",
        "instructions": (
            f"Resolve SWE-bench Lite instance {candidate.instance_id} for "
            f"{candidate.repo} at base commit {candidate.base_commit}.\n\n"
            f"Problem statement:\n{candidate.problem_statement}\n\n"
            "Make the code change that would resolve the issue. "
            "Do not modify global config. Avoid broad refactors."
        ),
        "files": [],
        "tests": [],
        "constraints": [
            "Keep the patch focused to the SWE-bench issue.",
            "Do not modify global Codex or OpenCode configuration.",
            "Official scoring is performed by the SWE-bench Docker harness after the patch is produced.",
        ],
        "acceptance": [
            "Patch directly addresses the problem statement.",
            "Official SWE-bench FAIL_TO_PASS tests should pass without PASS_TO_PASS regressions.",
        ],
        "context": {
            "benchmark": "SWE-bench Lite",
            "dataset": "SWE-bench/SWE-bench_Lite",
            "split": "test",
            "instance_id": candidate.instance_id,
            "repo": candidate.repo,
            "base_commit": candidate.base_commit,
            "version": candidate.version,
            "fail_to_pass": candidate.fail_to_pass,
            "pass_to_pass": candidate.pass_to_pass,
            "candidate_pool": "current-default-v1-expansion",
            "selection_rule": (
                "Screen cached-repo SWE-bench Lite candidates with explicit "
                "gpt-5.5/high Codex-only and keep official resolved instances."
            ),
        },
    }


def load_candidates(limit: int, exclude_repos: set[str]) -> list[Candidate]:
    dataset = load_dataset("SWE-bench/SWE-bench_Lite", split="test")
    candidates: list[Candidate] = []
    for row in dataset:
        if row["repo"] not in CACHED_REPOS:
            continue
        if row["repo"] in exclude_repos:
            continue
        if row["instance_id"] in EXISTING_SELECTED:
            continue
        files, changed = patch_stats(row["patch"])
        _, test_changed = patch_stats(row["test_patch"])
        candidates.append(
            Candidate(
                instance_id=row["instance_id"],
                repo=row["repo"],
                base_commit=row["base_commit"],
                version=row["version"],
                problem_statement=row["problem_statement"],
                fail_to_pass=row["FAIL_TO_PASS"],
                pass_to_pass=row["PASS_TO_PASS"],
                patch_changed_lines=changed,
                patch_files=files,
                test_patch_changed_lines=test_changed,
            )
        )
    candidates.sort(
        key=lambda c: (
            c.patch_changed_lines,
            c.patch_files,
            c.test_patch_changed_lines,
            c.repo,
            c.instance_id,
        )
    )
    return candidates[:limit]


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=False) + "\n")


def ensure_worktree(root: Path, candidate: Candidate, exp_dir: Path) -> None:
    work_dir = exp_dir / "work" / "codex-only"
    if work_dir.exists():
        return
    cache = project_state(root) / "swebench" / "repo-cache" / repo_key(candidate.repo)
    if not cache.exists():
        raise RuntimeError(f"missing repo cache for {candidate.repo}: {cache}")
    work_dir.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "clone", str(cache), str(work_dir)], cwd=root, check=True)
    checkout = subprocess.run(
        ["git", "-C", str(work_dir), "checkout", candidate.base_commit],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
    )
    if checkout.returncode != 0:
        subprocess.run(["git", "-C", str(cache), "fetch", "origin", candidate.base_commit], cwd=root, check=True)
        subprocess.run(["git", "-C", str(work_dir), "fetch", "origin", candidate.base_commit], cwd=root, check=True)
        subprocess.run(["git", "-C", str(work_dir), "checkout", candidate.base_commit], cwd=root, check=True)


def write_prediction_jsonl(root: Path, instance_id: str, patch_path: Path, output: Path) -> None:
    patch = patch_path.read_text()
    pred = {
        "instance_id": instance_id,
        "model_name_or_path": "mixmod-codex-only-current-default-v1-expansion",
        "model_patch": patch,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(pred) + "\n")


def eval_summary_path(root: Path, run_id: str) -> Path:
    name = f"mixmod-codex-only-current-default-v1-expansion.{run_id}.json"
    return project_state(root) / "swebench" / "current-default-v1-expansion" / "eval" / "official-summary" / name


def move_eval_summary(root: Path, run_id: str) -> Path:
    src = root / f"mixmod-codex-only-current-default-v1-expansion.{run_id}.json"
    dst = eval_summary_path(root, run_id)
    dst.parent.mkdir(parents=True, exist_ok=True)
    if src.exists():
        shutil.move(str(src), str(dst))
    return dst


def evaluate_patch(root: Path, candidate: Candidate, prediction: Path, run_id: str, log_dir: Path) -> tuple[bool, Path | None]:
    env = {
        "LD_LIBRARY_PATH": f"/run/opengl-driver/lib:/run/current-system/sw/lib:{os.environ.get('LD_LIBRARY_PATH', '')}",
    }
    log = log_dir / f"eval-{candidate.instance_id}.txt"
    code = run(
        [
            "swebench-eval",
            "--dataset_name",
            "SWE-bench/SWE-bench_Lite",
            "--split",
            "test",
            "--instance_ids",
            candidate.instance_id,
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
        env=env,
        log=log,
    )
    summary = move_eval_summary(root, run_id)
    if code != 0 or not summary.exists():
        return False, summary if summary.exists() else None
    data = read_json(summary)
    return candidate.instance_id in data.get("resolved_ids", []), summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target-resolved", type=int, default=7)
    parser.add_argument("--candidate-limit", type=int, default=30)
    parser.add_argument(
        "--exclude-repo",
        action="append",
        default=[],
        help="Repository name to skip while building the candidate queue. May be repeated.",
    )
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()

    root = args.root.resolve()
    state = project_state(root)
    pool = state / "swebench" / "current-default-v1-expansion"
    pool.mkdir(parents=True, exist_ok=True)
    log_dir = pool / "logs"

    exclude_repos = set(args.exclude_repo)
    candidates = load_candidates(args.candidate_limit, exclude_repos)
    write_json(pool / "queue.json", [c.__dict__ for c in candidates])

    state_path = pool / "screen-state.json"
    if state_path.exists():
        state = read_json(state_path)
    else:
        state = {"resolved": [], "attempted": []}

    attempted_ids = {item["instance_id"] for item in state.get("attempted", [])}
    resolved_ids = {item["instance_id"] for item in state.get("resolved", [])}

    print(
        f"queue={len(candidates)} attempted={len(attempted_ids)} "
        f"resolved={len(resolved_ids)} target={args.target_resolved} "
        f"exclude_repos={sorted(exclude_repos)}",
        flush=True,
    )

    for candidate in candidates:
        if len(resolved_ids) >= args.target_resolved:
            break
        if candidate.instance_id in attempted_ids:
            continue

        exp_name = f"swebench-current-default-v1-{safe_instance(candidate.instance_id)}"
        exp_dir = state / "experiments" / exp_name
        task_path = exp_dir / "task.json"
        print(f"SCREEN_START {candidate.instance_id} repo={candidate.repo} patch_lines={candidate.patch_changed_lines}", flush=True)

        exp_dir.mkdir(parents=True, exist_ok=True)
        write_json(task_path, make_task(candidate))
        ensure_worktree(root, candidate, exp_dir)

        codex_log = log_dir / f"codex-only-{candidate.instance_id}.txt"
        code = run(
            [
                str(root / "target" / "debug" / "mixmod"),
                "experiment",
                "record-codex-only",
                exp_name,
                "--task",
                str(task_path),
            ],
            cwd=root,
            log=codex_log,
        )

        metrics_path = exp_dir / "codex-only" / "metrics.json"
        patch_path = exp_dir / "codex-only" / "final.patch"
        metrics: dict[str, Any] = read_json(metrics_path) if metrics_path.exists() else {}
        patch_bytes = patch_path.stat().st_size if patch_path.exists() else 0
        resolved = False
        summary_path: Path | None = None

        if code == 0 and patch_bytes > 0:
            run_id = f"current-default-v1-codex-{safe_instance(candidate.instance_id)}"
            prediction = pool / "eval" / f"codex-only-{candidate.instance_id}.jsonl"
            write_prediction_jsonl(root, candidate.instance_id, patch_path, prediction)
            print(f"EVAL_START {candidate.instance_id} patch_bytes={patch_bytes}", flush=True)
            resolved, summary_path = evaluate_patch(root, candidate, prediction, run_id, log_dir)
        else:
            print(f"EVAL_SKIP {candidate.instance_id} codex_exit={code} patch_bytes={patch_bytes}", flush=True)

        attempted = {
            "instance_id": candidate.instance_id,
            "repo": candidate.repo,
            "base_commit": candidate.base_commit,
            "experiment": exp_name,
            "codex_exit": code,
            "patch_bytes": patch_bytes,
            "supervisor_input_tokens": metrics.get("supervisor_input_tokens"),
            "supervisor_output_tokens": metrics.get("supervisor_output_tokens"),
            "supervisor_total_tokens": metrics.get("supervisor_total_tokens"),
            "resolved": resolved,
            "official_summary": display_path(root, summary_path) if summary_path else None,
        }
        state.setdefault("attempted", []).append(attempted)
        attempted_ids.add(candidate.instance_id)
        if resolved:
            state.setdefault("resolved", []).append(attempted)
            resolved_ids.add(candidate.instance_id)
            print(f"SCREEN_RESOLVED {candidate.instance_id} resolved_count={len(resolved_ids)}", flush=True)
        else:
            print(f"SCREEN_NOT_RESOLVED {candidate.instance_id}", flush=True)
        write_json(state_path, state)

    print(f"SCREEN_DONE resolved={len(resolved_ids)} attempted={len(attempted_ids)}", flush=True)
    return 0 if len(resolved_ids) >= args.target_resolved else 2


if __name__ == "__main__":
    sys.exit(main())
