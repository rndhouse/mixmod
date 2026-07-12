"""Pier installed-agent wrapper for direct Codex DeepSWE baseline runs."""

from __future__ import annotations

import json
import shlex
from pathlib import PurePosixPath
from typing import Any

from pier.agents.installed.base import BaseInstalledAgent, with_prompt_template
from pier.environments.base import BaseEnvironment
from pier.models.agent.context import AgentContext
from pier.models.agent.install import AgentInstallSpec, InstallStep
from pier.models.agent.network import NetworkAllowlist
from pier.models.trial.paths import EnvironmentPaths

from scripts.deepswe_mixmod_agent import (
    INSTALL_DOMAINS,
    OPENAI_DOMAINS,
    PATH_SETUP,
    prepare_codex_auth,
)


class CodexAppAgent(BaseInstalledAgent):
    """Run Codex directly inside a Pier DeepSWE task container."""

    SUPPORTS_ATIF = False

    def __init__(
        self,
        *args: Any,
        codex_model: str = "gpt-5.5:high",
        codex_timeout_sec: int | str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.codex_model = codex_model
        self.timeout_sec = (
            int(codex_timeout_sec) if codex_timeout_sec not in (None, "") else None
        )

    @staticmethod
    def name() -> str:
        return "codex-direct"

    def get_version_command(self) -> str | None:
        return PATH_SETUP + "codex --version"

    def install_spec(self) -> AgentInstallSpec:
        root_run = (
            "set -euo pipefail; "
            "if command -v apt-get >/dev/null 2>&1; then "
            "apt-get update && "
            "DEBIAN_FRONTEND=noninteractive apt-get install -y "
            "bash ca-certificates curl git python3 ripgrep; "
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
            "npm install -g @openai/codex; "
            'mkdir -p "$HOME/.local/bin"; '
            'ln -sf "$(command -v codex)" "$HOME/.local/bin/codex"; '
            "codex --version"
        )
        return AgentInstallSpec(
            agent_name=self.name(),
            version=self._version,
            cache_key="codex-direct-deepswe-agent-v1",
            steps=[
                InstallStep(user="root", env={"DEBIAN_FRONTEND": "noninteractive"}, run=root_run),
                InstallStep(user="agent", run=node_run),
            ],
            verification_command=self.get_version_command(),
        )

    def network_allowlist(self) -> NetworkAllowlist:
        return NetworkAllowlist(domains=sorted(set(OPENAI_DOMAINS) | set(INSTALL_DOMAINS)))

    def populate_context_post_run(self, context: AgentContext) -> None:
        return None

    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        prompt_path = PurePosixPath("/tmp/deepswe-codex-prompt.md")
        summary_path = EnvironmentPaths.agent_dir / "codex-summary.json"
        await prepare_codex_auth(environment)
        await self.exec_as_agent(
            environment,
            command=(
                "python3 - <<'PY'\n"
                "from pathlib import Path\n"
                f"Path({prompt_path.as_posix()!r}).write_text({instruction!r} + '\\n')\n"
                "PY\n"
            ),
        )
        command = self._run_command(prompt_path, summary_path)
        await self.exec_as_agent(
            environment,
            command=command,
            env=self.build_process_env(),
            timeout_sec=self.timeout_sec,
        )
        await self._populate_context_from_summary(environment, summary_path, context)

    def _run_command(
        self,
        prompt_path: PurePosixPath,
        summary_path: PurePosixPath,
    ) -> str:
        model, effort = split_codex_model(self.codex_model)
        agent_dir = EnvironmentPaths.agent_dir
        stdout_path = agent_dir / "codex.exec.stdout.jsonl"
        stderr_path = agent_dir / "codex.exec.stderr.txt"
        metrics_path = agent_dir / "codex-metrics.json"
        prompt_artifact = agent_dir / "codex-prompt.md"
        last_message_path = agent_dir / "codex-last-message.md"
        sessions_dir = agent_dir / "codex-rollouts"
        return f"""set -euo pipefail
{PATH_SETUP}cd /app
mkdir -p {shlex.quote(agent_dir.as_posix())}
mkdir -p /tmp/codex-home
cp "$HOME/.codex/auth.json" /tmp/codex-home/auth.json
cp {shlex.quote(prompt_path.as_posix())} {shlex.quote(prompt_artifact.as_posix())}
git config user.name "Codex"
git config user.email "codex@example.invalid"
base_ref="$(git rev-parse HEAD)"
set +e
CODEX_HOME=/tmp/codex-home codex \\
  --ask-for-approval never \\
  exec \\
  --json \\
  --model {shlex.quote(model)} \\
  --sandbox danger-full-access \\
  --cd /app \\
  --config {shlex.quote(f'model_reasoning_effort="{effort}"')} \\
  --output-last-message {shlex.quote(last_message_path.as_posix())} \\
  - < {shlex.quote(prompt_path.as_posix())} \\
  > {shlex.quote(stdout_path.as_posix())} \\
  2> {shlex.quote(stderr_path.as_posix())}
codex_status=$?
set -e
mkdir -p {shlex.quote(sessions_dir.as_posix())}
if [ -d /tmp/codex-home/sessions ]; then
  cp -R /tmp/codex-home/sessions/. {shlex.quote(sessions_dir.as_posix())}/
fi
CODEX_STATUS="$codex_status" python3 - <<'PY'
import json
import os
from pathlib import Path

agent_dir = Path({agent_dir.as_posix()!r})
stdout_path = Path({stdout_path.as_posix()!r})
stderr_path = Path({stderr_path.as_posix()!r})
metrics_path = Path({metrics_path.as_posix()!r})
summary_path = Path({summary_path.as_posix()!r})
rollouts = sorted((agent_dir / "codex-rollouts").rglob("rollout-*.jsonl"))
status = int(os.environ.get("CODEX_STATUS", "1"))

def zero_usage():
    return {{
        "input_tokens": 0,
        "cached_input_tokens": 0,
        "output_tokens": 0,
        "reasoning_tokens": 0,
        "total_tokens": 0,
    }}

def snake_usage(value):
    return {{
        "input_tokens": int(value.get("input_tokens") or 0),
        "cached_input_tokens": int(value.get("cached_input_tokens") or 0),
        "output_tokens": int(value.get("output_tokens") or 0),
        "reasoning_tokens": int(value.get("reasoning_output_tokens") or value.get("reasoning_tokens") or 0),
        "total_tokens": int(value.get("total_tokens") or 0),
    }}

def camel_usage(value):
    return {{
        "input_tokens": int(value.get("inputTokens") or 0),
        "cached_input_tokens": int(value.get("cachedInputTokens") or 0),
        "output_tokens": int(value.get("outputTokens") or 0),
        "reasoning_tokens": int(value.get("reasoningOutputTokens") or value.get("reasoningTokens") or 0),
        "total_tokens": int(value.get("totalTokens") or 0),
    }}

def usage_from_value(value):
    payload = value.get("payload")
    if value.get("type") == "event_msg" and isinstance(payload, dict):
        if payload.get("type") == "token_count":
            info = payload.get("info") or {{}}
            total = info.get("total_token_usage")
            if isinstance(total, dict):
                return snake_usage(total)
    if value.get("type") == "token_count":
        info = value.get("info") or {{}}
        total = info.get("total_token_usage")
        if isinstance(total, dict):
            return snake_usage(total)
    if value.get("method") == "thread/tokenUsage/updated":
        params = value.get("params") or {{}}
        token_usage = params.get("tokenUsage") or {{}}
        total = token_usage.get("total")
        last = token_usage.get("last")
        if isinstance(total, dict):
            return camel_usage(total)
        if isinstance(last, dict):
            return camel_usage(last)
    return None

def usage_from_jsonl(path):
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

usage = zero_usage()
source = "none"
for rollout in rollouts:
    next_usage = usage_from_jsonl(rollout)
    if next_usage["total_tokens"]:
        for key, value in next_usage.items():
            usage[key] += value
        source = "codex_rollout_total_token_usage"

if not usage["total_tokens"]:
    next_usage = usage_from_jsonl(stdout_path)
    if next_usage["total_tokens"]:
        usage = next_usage
        source = "codex_stdout_total_token_usage"

metrics = {{
    "kind": "external-codex-baseline",
    "runner_mode": "codex-direct",
    "codex_exit_status": status,
    "codex_model": {model!r},
    "codex_reasoning_effort": {effort!r},
    "codex_input_tokens": usage["input_tokens"],
    "codex_cached_input_tokens": usage["cached_input_tokens"],
    "codex_output_tokens": usage["output_tokens"],
    "codex_reasoning_tokens": usage["reasoning_tokens"],
    "codex_total_tokens": usage["total_tokens"],
    "codex_token_usage": usage["total_tokens"],
    "codex_token_usage_source": source,
    "codex_rollout_count": len(rollouts),
    "stdout_bytes": stdout_path.stat().st_size if stdout_path.exists() else 0,
    "stderr_bytes": stderr_path.stat().st_size if stderr_path.exists() else 0,
    "final_status": "success" if status == 0 else "needs_review",
}}
metrics_path.write_text(json.dumps(metrics, indent=2, sort_keys=True) + "\\n")
summary_path.write_text(json.dumps(metrics, indent=2, sort_keys=True) + "\\n")
PY
if [ -n "$(git status --porcelain)" ]; then
  git add -A
  git commit -m "Codex solution"
elif [ "$(git rev-parse HEAD)" != "$base_ref" ]; then
  true
else
  git commit --allow-empty -m "Codex empty solution"
fi
exit "$codex_status"
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
        context.n_input_tokens = summary.get("codex_input_tokens")
        context.n_cache_tokens = summary.get("codex_cached_input_tokens")
        context.n_output_tokens = summary.get("codex_output_tokens")
        context.metadata = {"codex_direct": summary}


def split_codex_model(value: str) -> tuple[str, str]:
    """Split a model string such as `gpt-5.5:high` into model and effort."""
    if ":" in value:
        model, effort = value.rsplit(":", 1)
        return model, effort
    return value, "high"
