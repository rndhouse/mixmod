"""Pier installed-agent wrapper for Codex app-server DeepSWE screening."""

from __future__ import annotations

import json
import shlex
from pathlib import PurePosixPath
from typing import Any

from pier.agents.installed.base import BaseInstalledAgent, with_prompt_template
from pier.environments.base import BaseEnvironment
from pier.models.agent.context import AgentContext
from pier.models.agent.install import AgentInstallSpec
from pier.models.agent.network import NetworkAllowlist
from pier.models.trial.paths import EnvironmentPaths

from scripts.deepswe_mixmod_agent import (
    PATH_SETUP,
    mixmod_install_spec,
    mixmod_network_allowlist,
    prepare_codex_auth,
)


class CodexAppAgent(BaseInstalledAgent):
    """Run a Codex-only Mixmod baseline through Codex app-server."""

    SUPPORTS_ATIF = False

    def __init__(
        self,
        *args: Any,
        supervisor_model: str = "gpt-5.5:high",
        mixmod_command: str = "mixmod",
        mixmod_install_command: str | None = None,
        mixmod_timeout_sec: int | str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.supervisor_model = supervisor_model
        self.mixmod_command = mixmod_command
        self.mixmod_install_command = mixmod_install_command
        self.mixmod_timeout_sec = (
            int(mixmod_timeout_sec) if mixmod_timeout_sec not in (None, "") else None
        )

    @staticmethod
    def name() -> str:
        return "mixmod-codex-app"

    def get_version_command(self) -> str | None:
        return PATH_SETUP + f"{shlex.quote(self.mixmod_command)} --version"

    def install_spec(self) -> AgentInstallSpec:
        return mixmod_install_spec(
            self.name(),
            self._version,
            self.mixmod_command,
            self.mixmod_install_command,
        )

    def network_allowlist(self) -> NetworkAllowlist:
        return mixmod_network_allowlist()

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
        summary_path = EnvironmentPaths.agent_dir / "mixmod-codex-summary.json"

        task = {
            "title": "DeepSWE task",
            "instructions": instruction,
            "expect_patch": True,
            "files": [],
            "tests": [],
            "constraints": [
                "Solve the DeepSWE task from the public instruction only.",
                "Do not inspect /solution or verifier internals.",
                "Commit the final repository changes before exiting.",
            ],
            "acceptance": [
                "The committed patch should satisfy the DeepSWE verifier.",
            ],
            "context": {
                "benchmark": "DeepSWE",
                "dataset": "datacurve/deep-swe",
                "lane": "codex-app-server-screen",
            },
        }

        await prepare_codex_auth(environment)
        await self.exec_as_agent(
            environment,
            command=(
                f"cat > {shlex.quote(task_path.as_posix())} <<'JSON'\n"
                f"{json.dumps(task, indent=2)}\n"
                "JSON\n"
            ),
        )

        env = self.build_process_env(
            {
                "MIXMOD_DEBUG_COMMANDS": "1",
                "MIXMOD_CODEX_ONLY_SANDBOX": "danger-full-access",
                "MIXMOD_STATE_DIR": state_dir.as_posix(),
            }
        )
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
        return f"""set -euo pipefail
{PATH_SETUP}trap 'rm -f "$HOME/.codex/auth.json"' EXIT
mkdir -p {shlex.quote(EnvironmentPaths.agent_dir.as_posix())}
git config user.name "Mixmod"
git config user.email "mixmod@example.invalid"
{shlex.quote(self.mixmod_command)} init
python3 - <<'PY'
from pathlib import Path

supervisor_model = {self.supervisor_model!r}

def split_supervisor(value):
    if ":" in value:
        model, effort = value.rsplit(":", 1)
        return model, effort
    return value, "high"

def rewrite_toml(path):
    supervisor, effort = split_supervisor(supervisor_model)
    section = None
    lines = []
    for line in path.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped.strip("[]")
        if section in {{"supervisor", "codex_worker"}} and stripped.startswith("model = "):
            line = f'model = "{{supervisor}}"'
        elif section in {{"supervisor", "codex_worker"}} and stripped.startswith("reasoning_effort = "):
            line = f'reasoning_effort = "{{effort}}"'
        lines.append(line)
    path.write_text("\\n".join(lines) + "\\n")

for path in Path({state_dir.as_posix()!r}).glob("projects/*/config.toml"):
    rewrite_toml(path)
PY
{shlex.quote(self.mixmod_command)} experiment init deepswe --fixture .
exp_dir="$(find {shlex.quote(state_dir.as_posix())}/projects -path '*/experiments/deepswe' -type d | head -1)"
cp {shlex.quote(task_path.as_posix())} "$exp_dir/task.json"
{shlex.quote(self.mixmod_command)} experiment record-codex-only deepswe --task "$exp_dir/task.json" 2>&1 | tee {shlex.quote((EnvironmentPaths.agent_dir / "mixmod-codex.txt").as_posix())}
cp "$exp_dir/codex-only/metrics.json" {shlex.quote((EnvironmentPaths.agent_dir / "mixmod-codex-metrics.json").as_posix())} || true
cp "$exp_dir/codex-only/final.patch" {shlex.quote((EnvironmentPaths.agent_dir / "mixmod-codex-final.patch").as_posix())} || true
if [ -s "$exp_dir/codex-only/final.patch" ]; then
  git apply --whitespace=nowarn "$exp_dir/codex-only/final.patch"
fi
python3 - <<'PY'
import json
from pathlib import Path

agent_dir = Path({EnvironmentPaths.agent_dir.as_posix()!r})
metrics_path = agent_dir / "mixmod-codex-metrics.json"
metrics = json.loads(metrics_path.read_text()) if metrics_path.exists() else {{}}
summary = {{
    "codex_backend": metrics.get("codex_backend"),
    "codex_exit_status": metrics.get("codex_exit_status"),
    "supervisor_input_tokens": metrics.get("supervisor_input_tokens"),
    "supervisor_cached_input_tokens": metrics.get("supervisor_cached_input_tokens"),
    "supervisor_output_tokens": metrics.get("supervisor_output_tokens"),
    "supervisor_total_tokens": metrics.get("supervisor_total_tokens"),
    "final_status": metrics.get("final_status"),
}}
Path({summary_path.as_posix()!r}).write_text(json.dumps(summary, indent=2) + "\\n")
PY
if [ -n "$(git status --porcelain)" ]; then
  git add -A
  git commit -m "Codex app-server solution"
else
  git commit --allow-empty -m "Codex app-server empty solution"
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
        context.metadata = {"mixmod_codex_app": summary}
