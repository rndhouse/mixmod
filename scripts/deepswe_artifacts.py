"""Helpers for organizing DeepSWE Pier run artifacts."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
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

    command = [
        *_mixmod_bench_command(),
        "deepswe",
        "normalize-artifacts",
        "--pool",
        str(pool),
        "--job-dir",
        str(job_dir),
        "--records-json",
        "-",
    ]
    result = subprocess.run(
        command,
        cwd=_repo_root(),
        input=json.dumps(records),
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "mixmod-bench artifact normalization failed\n"
            f"command: {command!r}\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    value = json.loads(result.stdout or "[]")
    return value if isinstance(value, list) else []


def _mixmod_bench_command() -> list[str]:
    explicit = os.environ.get("MIXMOD_BENCH_BIN")
    if explicit:
        return [explicit]

    root = _repo_root()
    for candidate in [
        root / "target" / "debug" / "mixmod-bench",
        root / "target" / "release" / "mixmod-bench",
    ]:
        if candidate.exists():
            return [str(candidate)]

    installed = shutil.which("mixmod-bench")
    if installed:
        return [installed]

    return ["cargo", "run", "--quiet", "--bin", "mixmod-bench", "--"]


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[1]
