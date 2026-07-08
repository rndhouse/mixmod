#!/usr/bin/env python3
"""Compatibility wrapper for the explicit DeepSWE Mixmod runner."""

from __future__ import annotations

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.deepswe_run_mixmod_from_codex_only import main


if __name__ == "__main__":
    print(
        "warning: scripts/deepswe_run_mixmod_selected.py was renamed to "
        "scripts/deepswe_run_mixmod_from_codex_only.py",
        file=sys.stderr,
    )
    sys.exit(main())
