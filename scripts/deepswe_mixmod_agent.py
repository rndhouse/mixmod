"""Pier installed-agent wrapper for running Mixmod on DeepSWE tasks."""

from __future__ import annotations

import json
import os
import shlex
import tempfile
from pathlib import Path, PurePosixPath
from urllib.parse import urlparse
from typing import Any

from pier.agents.installed.base import BaseInstalledAgent, with_prompt_template
from pier.environments.base import BaseEnvironment
from pier.models.agent.context import AgentContext
from pier.models.agent.install import AgentInstallSpec, InstallStep
from pier.models.agent.network import NetworkAllowlist
from pier.models.trial.paths import EnvironmentPaths


OPENAI_DOMAINS = {
    "api.openai.com",
    "auth.openai.com",
    "chatgpt.com",
    "cdn.oaistatic.com",
}

OPENROUTER_DOMAINS = {
    "api.openrouter.ai",
    "openrouter.ai",
}

INSTALL_DOMAINS = {
    "github.com",
    "raw.githubusercontent.com",
    "registry.npmjs.org",
    "crates.io",
    "index.crates.io",
    "static.rust-lang.org",
}

PATH_SETUP = (
    'export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"\n'
    'if [ -s "$HOME/.nvm/nvm.sh" ]; then . "$HOME/.nvm/nvm.sh"; fi\n'
)
LOCAL_MIXMOD_COMMAND = PurePosixPath("/tmp/mixmod-local/mixmod")
ARTIFACTS_DIR = PurePosixPath("/logs/artifacts")
SNAPSHOT_SECONDS = 30


def csv_env_values_for_url(extra_url: str | None) -> list[str]:
    values = ["localhost", "127.0.0.1"]
    if extra_url:
        parsed = urlparse(extra_url)
        if parsed.hostname:
            values.append(parsed.hostname)
    return values


def merge_csv_env(env: dict[str, str], key: str, values: list[str]) -> None:
    parts = [part.strip() for part in env.get(key, "").split(",") if part.strip()]
    for value in values:
        if value and value not in parts:
            parts.append(value)
    env[key] = ",".join(parts)


class MixmodAgent(BaseInstalledAgent):
    """Run Mixmod with a GPT supervisor and OpenCode/Qwen worker inside Pier."""

    SUPPORTS_ATIF = False

    def __init__(
        self,
        *args: Any,
        supervisor_model: str = "gpt-5.5:high",
        worker_model: str = "llama.cpp/qwen/qwen3.6-27b",
        worker_backend: str = "opencode",
        strategy: str | None = None,
        supervisor_init: str = "compact",
        stop_after_first_worker: bool | str = False,
        stop_after_first_review: bool | str = False,
        stop_after_worker_turns: int | str | None = None,
        worker_target_patch_lines: int | str | None = None,
        worker_max_patch_lines: int | str | None = None,
        require_local: bool | str = True,
        mixmod_command: str = "mixmod",
        mixmod_install_command: str | None = None,
        local_mixmod_binary: str | None = None,
        worker_base_url: str | None = None,
        ollama_base_url: str | None = None,
        mixmod_timeout_sec: int | str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.supervisor_model = supervisor_model
        self.worker_model = worker_model
        self.worker_backend = worker_backend
        self.strategy = strategy
        self.supervisor_init = supervisor_init
        self.stop_after_first_worker = _truthy(stop_after_first_worker)
        self.stop_after_first_review = _truthy(stop_after_first_review)
        self.stop_after_worker_turns = (
            int(stop_after_worker_turns)
            if stop_after_worker_turns not in (None, "")
            else None
        )
        if sum(
            bool(value)
            for value in [
                self.stop_after_first_worker,
                self.stop_after_first_review,
                self.stop_after_worker_turns is not None,
            ]
        ) > 1:
            raise ValueError(
                "stop_after_first_worker, stop_after_first_review, and "
                "stop_after_worker_turns are mutually exclusive"
            )
        self.worker_target_patch_lines = (
            int(worker_target_patch_lines)
            if worker_target_patch_lines not in (None, "")
            else None
        )
        self.worker_max_patch_lines = (
            int(worker_max_patch_lines)
            if worker_max_patch_lines not in (None, "")
            else None
        )
        self.require_local = _truthy(require_local)
        self.mixmod_command = mixmod_command
        self.mixmod_install_command = mixmod_install_command
        self.local_mixmod_binary = (
            Path(local_mixmod_binary).expanduser().resolve()
            if local_mixmod_binary
            else None
        )
        self.container_mixmod_command = (
            LOCAL_MIXMOD_COMMAND.as_posix()
            if self.local_mixmod_binary
            else self.mixmod_command
        )
        self.worker_base_url = worker_base_url or ollama_base_url
        self.mixmod_timeout_sec = (
            int(mixmod_timeout_sec) if mixmod_timeout_sec not in (None, "") else None
        )

    @staticmethod
    def name() -> str:
        return "mixmod"

    def get_version_command(self) -> str | None:
        if self.local_mixmod_binary:
            return None
        return PATH_SETUP + f"{shlex.quote(self.mixmod_command)} --version"

    def install_spec(self) -> AgentInstallSpec:
        return mixmod_install_spec(
            self.name(),
            self._version,
            self.mixmod_command,
            self.mixmod_install_command,
            use_local_binary=bool(self.local_mixmod_binary),
        )

    def network_allowlist(self) -> NetworkAllowlist:
        return mixmod_network_allowlist(self.worker_base_url)

    def populate_context_post_run(self, context: AgentContext) -> None:
        return None

    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        task_path = PurePosixPath("/tmp/mixmod-deepswe-task.json")
        state_dir = PurePosixPath("/tmp/mixmod-state")
        summary_path = EnvironmentPaths.agent_dir / "mixmod-summary.json"

        task = {
            "title": "DeepSWE task",
            "instructions": instruction,
            "expect_patch": True,
            "files": [],
            "tests": [],
            "constraints": [
                "Solve the DeepSWE task from the public instruction only.",
                "Do not inspect /solution or verifier internals.",
                "Do not commit during Mixmod worker turns; leave changes as a repository diff.",
            ],
            "acceptance": [
                "The final patch should satisfy the DeepSWE verifier.",
            ],
            "context": {
                "benchmark": "DeepSWE",
                "dataset": "datacurve/deep-swe",
            },
        }
        await prepare_codex_auth(environment)
        if self.worker_model.split("/", 1)[0] == "openrouter":
            await prepare_opencode_auth(environment)
        if self.local_mixmod_binary:
            await prepare_local_mixmod_binary(environment, self.local_mixmod_binary)
        write_task = (
            f"cat > {shlex.quote(task_path.as_posix())} <<'JSON'\n"
            f"{json.dumps(task, indent=2)}\n"
            "JSON\n"
        )
        await self.exec_as_agent(environment, command=write_task)

        env = self.build_process_env()
        env.update(
            {
                "MIXMOD_CODEX_SUPERVISOR_SANDBOX": "danger-full-access",
                "MIXMOD_DEBUG_COMMANDS": "1",
                "MIXMOD_STATE_DIR": state_dir.as_posix(),
            }
        )
        if self.worker_base_url:
            env["MIXMOD_OPENCODE_BASE_URL"] = self.worker_base_url
            no_proxy_values = csv_env_values_for_url(self.worker_base_url)
            merge_csv_env(env, "NO_PROXY", no_proxy_values)
            merge_csv_env(env, "no_proxy", no_proxy_values)

        command = self._run_command(task_path, state_dir, summary_path)
        await self.exec_as_agent(
            environment,
            command=command,
            env=env,
            timeout_sec=self.mixmod_timeout_sec,
        )
        await self._populate_context_from_summary(environment, summary_path, context)

    def _run_command(
        self,
        task_path: PurePosixPath,
        state_dir: PurePosixPath,
        summary_path: PurePosixPath,
    ) -> str:
        run_default_args = [
            self.container_mixmod_command,
            "exec",
            "--task",
            task_path.as_posix(),
        ]
        run_default_args.extend(
            [
                "--supervisor-model",
                self.supervisor_model,
                "--worker-backend",
                self.worker_backend,
                "--worker-model",
                self.worker_model,
            ]
        )
        if self.strategy:
            run_default_args.extend(["--strategy", self.strategy])
        run_default_args.extend(["--supervisor-init", self.supervisor_init])
        if self.stop_after_first_worker:
            run_default_args.append("--stop-after-first-worker")
        if self.stop_after_first_review:
            run_default_args.append("--stop-after-first-review")
        if self.stop_after_worker_turns is not None:
            run_default_args.extend(
                ["--stop-after-worker-turns", str(self.stop_after_worker_turns)]
            )
        if self.worker_target_patch_lines is not None:
            run_default_args.extend(
                ["--worker-target-patch-lines", str(self.worker_target_patch_lines)]
            )
        if self.worker_max_patch_lines is not None:
            run_default_args.extend(
                ["--worker-max-patch-lines", str(self.worker_max_patch_lines)]
            )
        if not self.require_local:
            run_default_args.append("--no-require-local")
        quoted_run_default = " ".join(shlex.quote(arg) for arg in run_default_args)
        return f"""set -euo pipefail
{PATH_SETUP}cd /app
mkdir -p {shlex.quote(EnvironmentPaths.agent_dir.as_posix())}
mkdir -p {shlex.quote(ARTIFACTS_DIR.as_posix())}
git config user.name "Mixmod"
git config user.email "mixmod@example.invalid"
{shlex.quote(self.container_mixmod_command)} init
python3 - <<'PY'
import json
import os
from pathlib import Path

base_url = os.environ.get("MIXMOD_OPENCODE_BASE_URL")

if base_url:
    for path in Path({state_dir.as_posix()!r}).glob("projects/*/opencode.json"):
        data = json.loads(path.read_text())
        for provider in data.get("provider", {{}}).values():
            options = provider.setdefault("options", {{}})
            if options.get("baseURL"):
                options["baseURL"] = base_url
        path.write_text(json.dumps(data, indent=2) + "\\n")
PY
snapshot_mixmod_artifacts() {{
python3 - <<'PY' || true
import json
import shutil
import subprocess
from pathlib import Path

state_dir = Path({state_dir.as_posix()!r})
agent_dir = Path({EnvironmentPaths.agent_dir.as_posix()!r})
artifacts_dir = Path({ARTIFACTS_DIR.as_posix()!r})
summary_path = Path({summary_path.as_posix()!r})

agent_dir.mkdir(parents=True, exist_ok=True)
artifacts_dir.mkdir(parents=True, exist_ok=True)

def copy_if_exists(source: Path, target: Path) -> None:
    try:
        if source.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
    except OSError:
        pass

def write_command_output(command: list[str], target: Path) -> None:
    try:
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        if result.returncode == 0:
            tmp = target.with_suffix(target.suffix + ".tmp")
            tmp.write_bytes(result.stdout)
            tmp.replace(target)
    except OSError:
        pass

write_command_output(["git", "-C", "/app", "diff", "--binary", "HEAD"], artifacts_dir / "model.patch")
write_command_output(["git", "-C", "/app", "status", "--porcelain"], artifacts_dir / "git-status.txt")

runs = [path for path in state_dir.glob("projects/*/runs/run-*") if path.is_dir()]
if not runs:
    raise SystemExit(0)

run_dir = max(runs, key=lambda path: path.stat().st_mtime)
copy_if_exists(run_dir / "metrics.json", agent_dir / "mixmod-metrics.json")
copy_if_exists(run_dir / "final.patch", agent_dir / "mixmod-final.patch")
copy_if_exists(run_dir / "report.md", agent_dir / "mixmod-report.md")
copy_if_exists(
    run_dir / "supervision-loop-summary.json",
    agent_dir / "supervision-loop-summary.json",
)
copy_if_exists(run_dir / "supervisor-feedback.jsonl", agent_dir / "supervisor-feedback.jsonl")
for name in ["task.json", "worker-brief.json", "worker-task.json"]:
    copy_if_exists(run_dir / name, agent_dir / name)

run_logs = run_dir / "logs"
if run_logs.exists():
    target_logs = agent_dir / "logs"
    target_logs.mkdir(parents=True, exist_ok=True)
    for source in sorted(run_logs.glob("codex*")):
        if source.is_file():
            copy_if_exists(source, target_logs / source.name)

def worker_dir_sort_key(path: Path) -> tuple[int, int, str]:
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

worker_root = run_dir / "worker-runs"
if worker_root.exists():
    for worker_dir in sorted(
        (path for path in worker_root.iterdir() if path.is_dir()),
        key=worker_dir_sort_key,
    ):
        target = agent_dir / "worker-runs" / worker_dir.relative_to(worker_root)
        target.mkdir(parents=True, exist_ok=True)
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
        ]:
            copy_if_exists(worker_dir / name, target / name)
        logs = worker_dir / "logs"
        if logs.exists():
            target_logs = target / "logs"
            target_logs.mkdir(parents=True, exist_ok=True)
            for name in [
                "opencode.events.jsonl",
                "opencode.stdout.txt",
                "opencode.stderr.txt",
                "heartbeat.jsonl",
            ]:
                copy_if_exists(logs / name, target_logs / name)

metrics_path = agent_dir / "mixmod-metrics.json"
metrics = json.loads(metrics_path.read_text()) if metrics_path.exists() else {{}}

def load_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return {{}}

def feedback_records(path: Path) -> list[dict]:
    records = []
    try:
        for line in path.read_text().splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    except OSError:
        pass
    return records

def sum_field(records: list[dict], field: str) -> int | None:
    values = [record.get(field) for record in records]
    values = [value for value in values if isinstance(value, int)]
    if not values:
        return None
    return sum(values)

def sum_float_field(records: list[dict], field: str) -> float | None:
    values = [record.get(field) for record in records]
    values = [value for value in values if isinstance(value, (int, float))]
    if not values:
        return None
    return float(sum(values))

def any_bool(values: list[object]) -> bool | None:
    bools = [value for value in values if isinstance(value, bool)]
    if not bools:
        return None
    return any(bools)

def file_len(path: Path) -> int | None:
    try:
        return path.stat().st_size
    except OSError:
        return None

def last_jsonl_record(path: Path) -> dict:
    try:
        lines = path.read_text().splitlines()
    except OSError:
        return {{}}
    for line in reversed(lines):
        line = line.strip()
        if not line:
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    return {{}}

def jsonl_records(path: Path) -> list[dict]:
    records = []
    try:
        lines = path.read_text().splitlines()
    except OSError:
        return records
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            records.append(value)
    return records

def worker_control_records(worker_dir: Path, worker_metric: dict) -> list[dict]:
    records = jsonl_records(worker_dir / "supervisor-control.jsonl")
    if records:
        return records
    events = worker_metric.get("supervisor_control_events")
    if isinstance(events, list):
        return [event for event in events if isinstance(event, dict)]
    return []

def string_values(records: list[dict], field: str) -> list[str]:
    values = []
    for record in records:
        value = record.get(field)
        if isinstance(value, str) and value.strip():
            values.append(value)
    return values

feedback = feedback_records(agent_dir / "supervisor-feedback.jsonl")
worker_dirs = sorted(
    (path for path in (agent_dir / "worker-runs").glob("*") if path.is_dir()),
    key=worker_dir_sort_key,
)
worker_metric_records = []
for worker_dir in worker_dirs:
    worker_metrics_path = worker_dir / "metrics.json"
    if not worker_metrics_path.exists():
        continue
    worker_metric = load_json(worker_metrics_path)
    if worker_metric:
        worker_metric_records.append((worker_dir, worker_metric))
worker_metrics = [item for _, item in worker_metric_records]
latest_completed_worker_dir = (
    worker_metric_records[-1][0]
    if worker_metric_records
    else None
)
latest_completed_worker = (
    worker_metric_records[-1][1]
    if worker_metric_records
    else {{}}
)
latest_worker_dir = worker_dirs[-1] if worker_dirs else None
latest_worker = (
    load_json(latest_worker_dir / "metrics.json")
    if latest_worker_dir and (latest_worker_dir / "metrics.json").exists()
    else {{}}
)
latest_heartbeat = (
    last_jsonl_record(latest_worker_dir / "logs" / "heartbeat.jsonl")
    if latest_worker_dir
    else {{}}
)
latest_worker_patch_comparison = (
    load_json(latest_worker_dir / "patch-comparison.json")
    if latest_worker_dir
    else {{}}
)
worker_patch_comparisons = [
    load_json(worker_dir / "patch-comparison.json")
    for worker_dir in worker_dirs
]
worker_patch_comparisons = [
    item for item in worker_patch_comparisons if item
]
observed_patch_observation_count = sum(
    1
    for item in worker_patch_comparisons
    if item.get("observations")
)
worker_metric_by_dir = dict(worker_metric_records)
latest_worker_controls = (
    worker_control_records(latest_worker_dir, latest_worker)
    if latest_worker_dir
    else []
)
all_worker_controls = []
for worker_dir in worker_dirs:
    all_worker_controls.extend(
        worker_control_records(worker_dir, worker_metric_by_dir.get(worker_dir, {{}}))
    )
latest_worker_control_actions = string_values(latest_worker_controls, "action")
latest_worker_control_risks = string_values(latest_worker_controls, "risk")
all_worker_control_actions = string_values(all_worker_controls, "action")
all_worker_control_risks = string_values(all_worker_controls, "risk")
all_worker_control_interrupts = [
    action for action in all_worker_control_actions if action.startswith("interrupt")
]
patch_path = artifacts_dir / "model.patch"
try:
    patch_text = patch_path.read_text()
except OSError:
    patch_text = ""

summary = {{
    "snapshot_status": "final" if metrics else "in_progress_or_interrupted",
    "strategy": metrics.get("strategy"),
    "supervisor_takeover": metrics.get("supervisor_takeover")
        if metrics.get("supervisor_takeover") is not None
        else any(
            (record.get("feedback") or {{}}).get("action") == "take_over"
            for record in feedback
        ),
    "supervisor_compaction_count": metrics.get("supervisor_compaction_count")
        if metrics.get("supervisor_compaction_count") is not None
        else sum(1 for record in feedback if record.get("type") == "supervisor_compaction"),
    "supervisor_direct_finish": metrics.get("supervisor_direct_finish"),
    "worker_backend": metrics.get("worker_backend") or latest_completed_worker.get("worker_backend"),
    "supervisor_stdout_bytes": file_len(agent_dir / "logs" / "codex.stdout.jsonl"),
    "supervisor_stderr_bytes": file_len(agent_dir / "logs" / "codex.stderr.txt"),
    "supervisor_input_tokens": metrics.get("supervisor_input_tokens") or sum_field(feedback, "supervisor_input_tokens"),
    "supervisor_cached_input_tokens": metrics.get("supervisor_cached_input_tokens") or sum_field(feedback, "supervisor_cached_input_tokens"),
    "supervisor_output_tokens": metrics.get("supervisor_output_tokens") or sum_field(feedback, "supervisor_output_tokens"),
    "supervisor_reasoning_tokens": metrics.get("supervisor_reasoning_tokens") or sum_field(feedback, "supervisor_reasoning_tokens"),
    "supervisor_total_tokens": metrics.get("supervisor_total_tokens") or sum_field(feedback, "supervisor_total_tokens"),
    "worker_input_tokens": metrics.get("worker_input_tokens") or sum_field(worker_metrics, "worker_input_tokens"),
    "worker_cached_input_tokens": metrics.get("worker_cached_input_tokens") or sum_field(worker_metrics, "worker_cached_input_tokens"),
    "worker_cache_write_tokens": metrics.get("worker_cache_write_tokens") or sum_field(worker_metrics, "worker_cache_write_tokens"),
    "worker_output_tokens": metrics.get("worker_output_tokens") or sum_field(worker_metrics, "worker_output_tokens"),
    "worker_reasoning_tokens": metrics.get("worker_reasoning_tokens") or sum_field(worker_metrics, "worker_reasoning_tokens"),
    "worker_total_tokens": metrics.get("worker_total_tokens") or sum_field(worker_metrics, "worker_total_tokens"),
    "worker_reported_cost_usd": metrics.get("worker_reported_cost_usd") or sum_float_field(worker_metrics, "worker_reported_cost_usd"),
    "worker_token_step_count": metrics.get("worker_token_step_count") or sum_field(worker_metrics, "worker_token_step_count"),
    "worker_token_usage_source": metrics.get("worker_token_usage_source")
        or latest_completed_worker.get("worker_token_usage_source"),
    "worker_token_usage_scope": metrics.get("worker_token_usage_scope")
        or latest_completed_worker.get("worker_token_usage_scope"),
    "worker_token_usage_comparable": metrics.get("worker_token_usage_comparable")
        if metrics.get("worker_token_usage_comparable") is not None
        else (
            all(
                item.get("worker_token_usage_comparable") is True
                for item in worker_metrics
            )
            if worker_metrics
            else None
        ),
    "codex_calls": metrics.get("codex_calls") or len(feedback) or None,
    "opencode_calls": metrics.get("opencode_calls") or len(worker_dirs) or len(worker_metrics) or None,
    "worker_runs_observed": len(worker_dirs) or None,
    "worker_runs_completed": len(worker_metrics) or None,
    "worker_runs_incomplete": (
        max(len(worker_dirs) - len(worker_metrics), 0)
        if worker_dirs
        else None
    ),
    "final_status": metrics.get("final_status"),
    "final_verdict": metrics.get("final_verdict"),
    "latest_supervisor_label": feedback[-1].get("label") if feedback else None,
    "latest_supervisor_action": (
        (feedback[-1].get("feedback") or {{}}).get("action")
        if feedback
        else None
    ),
    "latest_worker_run": latest_worker.get("opencode_session_label"),
    "latest_worker_dir": latest_worker_dir.name if latest_worker_dir else None,
    "latest_worker_completed": bool(latest_worker),
    "latest_worker_session_reused": latest_worker.get("worker_session_reused"),
    "latest_worker_context_overflow_count": latest_worker.get("context_overflow_count"),
    "latest_worker_token_peak": latest_worker.get("worker_session_token_peak"),
    "latest_worker_heartbeat_status": latest_heartbeat.get("status"),
    "latest_worker_heartbeat_elapsed_ms": latest_heartbeat.get("elapsed_ms"),
    "latest_worker_last_output_age_ms": latest_heartbeat.get("last_output_age_ms"),
    "latest_worker_stdout_bytes": (
        file_len(latest_worker_dir / "logs" / "opencode.stdout.txt")
        if latest_worker_dir
        else None
    ),
    "latest_worker_events_bytes": (
        file_len(latest_worker_dir / "logs" / "opencode.events.jsonl")
        if latest_worker_dir
        else None
    ),
    "latest_worker_stderr_bytes": (
        file_len(latest_worker_dir / "logs" / "opencode.stderr.txt")
        if latest_worker_dir
        else None
    ),
    "latest_worker_changes_patch_bytes": (
        file_len(latest_worker_dir / "changes.patch")
        if latest_worker_dir
        else None
    ),
    "latest_worker_worktree_patch_bytes": (
        file_len(latest_worker_dir / "worktree.patch")
        if latest_worker_dir
        else None
    ),
    "latest_worker_partial_patch_bytes": (
        file_len(latest_worker_dir / "partial.patch")
        if latest_worker_dir
        else None
    ),
    "latest_worker_patch_comparison_bytes": (
        file_len(latest_worker_dir / "patch-comparison.json")
        if latest_worker_dir
        else None
    ),
    "latest_worker_patch_observations": (
        latest_worker_patch_comparison.get("observations")
        if latest_worker_patch_comparison
        else None
    ),
    "latest_worker_control_log_bytes": (
        file_len(latest_worker_dir / "supervisor-control.jsonl")
        if latest_worker_dir
        else None
    ),
    "latest_worker_supervisor_control_count": (
        len(latest_worker_controls)
        if latest_worker_dir
        else None
    ),
    "latest_worker_supervisor_control_actions": (
        latest_worker_control_actions or None
    ),
    "latest_worker_supervisor_control_risks": (
        latest_worker_control_risks or None
    ),
    "latest_worker_supervisor_control_last_action": (
        latest_worker_control_actions[-1]
        if latest_worker_control_actions
        else None
    ),
    "latest_worker_interrupted_by_supervisor": (
        any(action.startswith("interrupt") for action in latest_worker_control_actions)
        if latest_worker_dir
        else None
    ),
    "latest_completed_worker_run": latest_completed_worker.get("opencode_session_label"),
    "latest_completed_worker_dir": (
        latest_completed_worker_dir.name
        if latest_completed_worker_dir
        else None
    ),
    "latest_completed_worker_session_reused": latest_completed_worker.get("worker_session_reused"),
    "latest_completed_worker_context_overflow_count": latest_completed_worker.get("context_overflow_count"),
    "latest_completed_worker_token_peak": latest_completed_worker.get("worker_session_token_peak"),
    "context_overflow_count": sum(
        item.get("context_overflow_count", 0)
        for item in worker_metrics
        if isinstance(item.get("context_overflow_count"), int)
    ) if worker_metrics else None,
    "observed_supervisor_control_count": (
        len(all_worker_controls) or None
    ),
    "observed_supervisor_control_actions": (
        all_worker_control_actions or None
    ),
    "observed_supervisor_control_risks": (
        all_worker_control_risks or None
    ),
    "observed_supervisor_control_interrupts": (
        len(all_worker_control_interrupts) or None
    ),
    "observed_patch_observation_count": (
        observed_patch_observation_count or None
    ),
    "worker_session_token_peak": max(
        [
            item.get("worker_session_token_peak")
            for item in worker_metrics
            if isinstance(item.get("worker_session_token_peak"), int)
        ],
        default=None,
    ),
    "local_inference_verified": metrics.get("local_inference_verified")
        if metrics.get("local_inference_verified") is not None
        else any_bool([item.get("local_inference_verified") for item in worker_metrics]),
    "gpu_activity_observed": metrics.get("gpu_activity_observed")
        if metrics.get("gpu_activity_observed") is not None
        else any_bool([item.get("gpu_activity_observed") for item in worker_metrics]),
    "local_worker_reasoning_trace_bytes": metrics.get("local_worker_reasoning_trace_bytes")
        or sum(
            item.get("reasoning_trace_bytes", 0)
            for item in worker_metrics
            if isinstance(item.get("reasoning_trace_bytes"), int)
        )
        or None,
    "local_worker_reasoning_trace_event_count": metrics.get("local_worker_reasoning_trace_event_count")
        or sum(
            item.get("reasoning_trace_event_count", 0)
            for item in worker_metrics
            if isinstance(item.get("reasoning_trace_event_count"), int)
        )
        or None,
    "patch_bytes": len(patch_text.encode()),
    "patch_lines": patch_text.count("\\n"),
    "repository_patch_observed": bool(patch_text.strip()),
}}
summary_path.write_text(json.dumps(summary, indent=2) + "\\n")
PY
}}
stop_snapshot_loop() {{
  if [ -n "${{snapshot_pid:-}}" ]; then
    kill "$snapshot_pid" 2>/dev/null || true
    wait "$snapshot_pid" 2>/dev/null || true
    snapshot_pid=""
  fi
}}
trap 'stop_snapshot_loop; rm -f "$HOME/.codex/auth.json" "$HOME/.local/share/opencode/auth.json"' EXIT
snapshot_mixmod_artifacts
while true; do
  sleep {SNAPSHOT_SECONDS}
  snapshot_mixmod_artifacts
done &
snapshot_pid="$!"
set +e
{quoted_run_default} 2>&1 | tee {shlex.quote((EnvironmentPaths.agent_dir / "mixmod.txt").as_posix())}
mixmod_exit="${{PIPESTATUS[0]}}"
set -e
stop_snapshot_loop
snapshot_mixmod_artifacts
if [ -n "$(git status --porcelain)" ]; then
  git add -A
  git commit -m "Mixmod solution"
else
  git commit --allow-empty -m "Mixmod empty solution"
fi
if [ "$mixmod_exit" -ne 0 ]; then
  exit "$mixmod_exit"
fi
"""

    async def _populate_context_from_summary(
        self,
        environment: BaseEnvironment,
        summary_path: PurePosixPath,
        context: AgentContext,
    ) -> None:
        result = await environment.exec(command=f"cat {shlex.quote(summary_path.as_posix())}")
        if result.return_code != 0 or not result.stdout:
            return
        try:
            summary = json.loads(result.stdout)
        except json.JSONDecodeError:
            return
        context.n_input_tokens = summary.get("supervisor_input_tokens")
        context.n_cache_tokens = summary.get("supervisor_cached_input_tokens")
        context.n_output_tokens = summary.get("supervisor_output_tokens")
        context.metadata = {"mixmod": summary}


def _truthy(value: bool | str) -> bool:
    if isinstance(value, bool):
        return value
    return value.strip().lower() not in {"", "0", "false", "no", "off"}


def mixmod_network_allowlist(extra_url: str | None = None) -> NetworkAllowlist:
    domains = set(OPENAI_DOMAINS)
    domains.update(OPENROUTER_DOMAINS)
    domains.update(INSTALL_DOMAINS)
    if extra_url:
        parsed = urlparse(extra_url)
        if parsed.hostname:
            domains.add(parsed.hostname)
    return NetworkAllowlist(domains=sorted(domains))


def mixmod_install_spec(
    agent_name: str,
    version: str | None,
    mixmod_command: str,
    mixmod_install_command: str | None,
    *,
    use_local_binary: bool = False,
) -> AgentInstallSpec:
    root_run = (
        "set -euo pipefail; "
        "if command -v apt-get >/dev/null 2>&1; then "
        "apt-get update && "
        "DEBIAN_FRONTEND=noninteractive apt-get install -y "
        "bash build-essential ca-certificates curl git libssl-dev pkg-config "
        "python3 ripgrep rustc cargo; "
        "fi"
    )
    node_run = (
        "set -euo pipefail; "
        'if [ ! -s "$HOME/.nvm/nvm.sh" ]; then '
        "curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.2/install.sh | bash; "
        "fi; "
        '. "$HOME/.nvm/nvm.sh"; '
        "nvm install 22; "
        "nvm alias default 22; "
        "npm install -g @openai/codex opencode-ai; "
        'mkdir -p "$HOME/.local/bin"; '
        'ln -sf "$(command -v codex)" "$HOME/.local/bin/codex"; '
        'ln -sf "$(command -v opencode)" "$HOME/.local/bin/opencode"; '
        "codex --version; "
        "opencode --version"
    )
    if use_local_binary:
        mixmod_run = (
            "set -euo pipefail; "
            f"{PATH_SETUP} "
            "echo 'using local Mixmod binary uploaded at runtime'; "
            "codex --version; "
            "opencode --version"
        )
        cache_key = "mixmod-deepswe-agent-v7-local"
        verification_command = f"{PATH_SETUP}codex --version && opencode --version"
    else:
        mixmod_run = (
            "set -euo pipefail; "
            f"{PATH_SETUP} "
            f"{mixmod_install_command or _default_mixmod_install(mixmod_command)}; "
            f"{shlex.quote(mixmod_command)} --version; "
            "codex --version; "
            "opencode --version"
        )
        cache_key = "mixmod-deepswe-agent-v7"
        verification_command = f"{PATH_SETUP}{shlex.quote(mixmod_command)} --version"
    return AgentInstallSpec(
        agent_name=agent_name,
        version=version,
        cache_key=cache_key,
        steps=[
            InstallStep(user="root", env={"DEBIAN_FRONTEND": "noninteractive"}, run=root_run),
            InstallStep(user="agent", run=node_run),
            InstallStep(user="agent", run=mixmod_run),
        ],
        verification_command=verification_command,
    )


def _default_mixmod_install(mixmod_command: str) -> str:
    return (
        f"if ! command -v {shlex.quote(mixmod_command)} >/dev/null 2>&1; then "
        "cargo install --git https://github.com/rndhouse/mixmod --locked; "
        "fi"
    )


async def prepare_codex_auth(environment: BaseEnvironment) -> None:
    explicit = os.environ.get("CODEX_AUTH_JSON_PATH")
    auth_path = Path(explicit).expanduser() if explicit else Path.home() / ".codex" / "auth.json"
    remote_auth_path = PurePosixPath("/tmp/mixmod-codex-auth.json")
    if auth_path.is_file():
        await environment.upload_file(auth_path, remote_auth_path)
        if environment.default_user is not None:
            await environment.exec(
                command=f"chown {environment.default_user} {shlex.quote(remote_auth_path.as_posix())}",
                user="root",
            )
        await environment.exec(
            command=(
                'mkdir -p "$HOME/.codex" && '
                f"cp {shlex.quote(remote_auth_path.as_posix())} "
                '"$HOME/.codex/auth.json" && '
                f"rm -f {shlex.quote(remote_auth_path.as_posix())}"
            )
        )
        return

    api_key = os.environ.get("OPENAI_API_KEY")
    if api_key:
        with tempfile.NamedTemporaryFile("w", delete=False) as handle:
            json.dump({"OPENAI_API_KEY": api_key}, handle, indent=2)
            handle.write("\n")
            temp_auth = Path(handle.name)
        try:
            await environment.upload_file(temp_auth, remote_auth_path)
        finally:
            temp_auth.unlink(missing_ok=True)
        if environment.default_user is not None:
            await environment.exec(
                command=f"chown {environment.default_user} {shlex.quote(remote_auth_path.as_posix())}",
                user="root",
            )
        await environment.exec(
            command=(
                'mkdir -p "$HOME/.codex" && '
                f"mv {shlex.quote(remote_auth_path.as_posix())} "
                '"$HOME/.codex/auth.json"'
            )
        )
        return

    raise RuntimeError(
        "Codex auth is required; set OPENAI_API_KEY, CODEX_AUTH_JSON_PATH, "
        "or provide ~/.codex/auth.json on the Pier host"
    )


async def prepare_local_mixmod_binary(
    environment: BaseEnvironment, binary_path: Path
) -> None:
    if not binary_path.is_file():
        raise FileNotFoundError(f"local Mixmod binary not found: {binary_path}")
    await environment.exec(
        command=f"mkdir -p {shlex.quote(LOCAL_MIXMOD_COMMAND.parent.as_posix())}"
    )
    await environment.upload_file(binary_path, LOCAL_MIXMOD_COMMAND)
    if environment.default_user is not None:
        await environment.exec(
            command=(
                f"chown {environment.default_user} "
                f"{shlex.quote(LOCAL_MIXMOD_COMMAND.as_posix())}"
            ),
            user="root",
        )
    await environment.exec(
        command=(
            f"chmod 755 {shlex.quote(LOCAL_MIXMOD_COMMAND.as_posix())} && "
            f"{shlex.quote(LOCAL_MIXMOD_COMMAND.as_posix())} --version"
        )
    )


async def prepare_opencode_auth(environment: BaseEnvironment) -> None:
    explicit = os.environ.get("OPENCODE_AUTH_JSON_PATH")
    auth_path = (
        Path(explicit).expanduser()
        if explicit
        else Path.home() / ".local" / "share" / "opencode" / "auth.json"
    )
    remote_auth_path = PurePosixPath("/tmp/mixmod-opencode-auth.json")
    if auth_path.is_file():
        await environment.upload_file(auth_path, remote_auth_path)
        if environment.default_user is not None:
            await environment.exec(
                command=f"chown {environment.default_user} {shlex.quote(remote_auth_path.as_posix())}",
                user="root",
            )
        await environment.exec(
            command=(
                'mkdir -p "$HOME/.local/share/opencode" && '
                f"cp {shlex.quote(remote_auth_path.as_posix())} "
                '"$HOME/.local/share/opencode/auth.json" && '
                'chmod 600 "$HOME/.local/share/opencode/auth.json" && '
                f"rm -f {shlex.quote(remote_auth_path.as_posix())}"
            )
        )
        return

    api_key = os.environ.get("OPENROUTER_API_KEY")
    if api_key:
        with tempfile.NamedTemporaryFile("w", delete=False) as handle:
            json.dump({"openrouter": {"type": "api", "key": api_key}}, handle, indent=2)
            handle.write("\n")
            temp_auth = Path(handle.name)
        try:
            await environment.upload_file(temp_auth, remote_auth_path)
        finally:
            temp_auth.unlink(missing_ok=True)
        if environment.default_user is not None:
            await environment.exec(
                command=f"chown {environment.default_user} {shlex.quote(remote_auth_path.as_posix())}",
                user="root",
            )
        await environment.exec(
            command=(
                'mkdir -p "$HOME/.local/share/opencode" && '
                f"mv {shlex.quote(remote_auth_path.as_posix())} "
                '"$HOME/.local/share/opencode/auth.json" && '
                'chmod 600 "$HOME/.local/share/opencode/auth.json"'
            )
        )
        return

    raise RuntimeError(
        "OpenRouter auth is required for OpenCode OpenRouter workers; set "
        "OPENROUTER_API_KEY, OPENCODE_AUTH_JSON_PATH, or provide "
        "~/.local/share/opencode/auth.json on the Pier host"
    )
