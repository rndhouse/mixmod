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


class MixmodAgent(BaseInstalledAgent):
    """Run Mixmod with a GPT supervisor and OpenCode/Qwen worker inside Pier."""

    SUPPORTS_ATIF = False

    def __init__(
        self,
        *args: Any,
        supervisor_model: str = "gpt-5.5:high",
        worker_model: str = "mixmod-local-ollama/qwen3.6:27b",
        worker_backend: str = "opencode",
        require_local: bool | str = True,
        mixmod_command: str = "mixmod",
        mixmod_install_command: str | None = None,
        ollama_base_url: str | None = None,
        mixmod_timeout_sec: int | str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.supervisor_model = supervisor_model
        self.worker_model = worker_model
        self.worker_backend = worker_backend
        self.require_local = _truthy(require_local)
        self.mixmod_command = mixmod_command
        self.mixmod_install_command = mixmod_install_command
        self.ollama_base_url = ollama_base_url
        self.mixmod_timeout_sec = (
            int(mixmod_timeout_sec) if mixmod_timeout_sec not in (None, "") else None
        )

    @staticmethod
    def name() -> str:
        return "mixmod"

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
        return mixmod_network_allowlist(self.ollama_base_url)

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
                "Commit the final repository changes before exiting.",
            ],
            "acceptance": [
                "The committed patch should satisfy the DeepSWE verifier.",
            ],
            "context": {
                "benchmark": "DeepSWE",
                "dataset": "datacurve/deep-swe",
            },
        }
        await prepare_codex_auth(environment)
        if self.worker_model.split("/", 1)[0] == "openrouter":
            await prepare_opencode_auth(environment)
        write_task = (
            f"cat > {shlex.quote(task_path.as_posix())} <<'JSON'\n"
            f"{json.dumps(task, indent=2)}\n"
            "JSON\n"
        )
        await self.exec_as_agent(environment, command=write_task)

        env = self.build_process_env()
        env.update(
            {
                "MIXMOD_DEBUG_COMMANDS": "1",
                "MIXMOD_STATE_DIR": state_dir.as_posix(),
            }
        )
        if self.ollama_base_url:
            env["MIXMOD_OPENCODE_BASE_URL"] = self.ollama_base_url

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
            self.mixmod_command,
            "experiment",
            "run-default",
            "deepswe",
        ]
        if self.require_local:
            run_default_args.append("--require-local")
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
        quoted_run_default = " ".join(shlex.quote(arg) for arg in run_default_args)
        return f"""set -euo pipefail
{PATH_SETUP}trap 'rm -f "$HOME/.codex/auth.json" "$HOME/.local/share/opencode/auth.json"' EXIT
mkdir -p {shlex.quote(EnvironmentPaths.agent_dir.as_posix())}
git config user.name "Mixmod"
git config user.email "mixmod@example.invalid"
{shlex.quote(self.mixmod_command)} init
{shlex.quote(self.mixmod_command)} experiment init deepswe --fixture .
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
exp_dir="$(find {shlex.quote(state_dir.as_posix())}/projects -path '*/experiments/deepswe' -type d | head -1)"
cp {shlex.quote(task_path.as_posix())} "$exp_dir/task.json"
{quoted_run_default} 2>&1 | tee {shlex.quote((EnvironmentPaths.agent_dir / "mixmod.txt").as_posix())}
cp "$exp_dir/default/metrics.json" {shlex.quote((EnvironmentPaths.agent_dir / "mixmod-metrics.json").as_posix())} || true
cp "$exp_dir/default/final.patch" {shlex.quote((EnvironmentPaths.agent_dir / "mixmod-final.patch").as_posix())} || true
if [ -s "$exp_dir/default/final.patch" ]; then
  git apply --whitespace=nowarn "$exp_dir/default/final.patch"
fi
python3 - <<'PY'
import json
from pathlib import Path

agent_dir = Path({EnvironmentPaths.agent_dir.as_posix()!r})
metrics_path = agent_dir / "mixmod-metrics.json"
metrics = json.loads(metrics_path.read_text()) if metrics_path.exists() else {{}}
summary = {{
    "worker_backend": metrics.get("worker_backend"),
    "supervisor_input_tokens": metrics.get("supervisor_input_tokens"),
    "supervisor_cached_input_tokens": metrics.get("supervisor_cached_input_tokens"),
    "supervisor_output_tokens": metrics.get("supervisor_output_tokens"),
    "supervisor_total_tokens": metrics.get("supervisor_total_tokens"),
    "codex_calls": metrics.get("codex_calls"),
    "opencode_calls": metrics.get("opencode_calls"),
    "final_status": metrics.get("final_status"),
    "final_verdict": metrics.get("final_verdict"),
    "local_inference_verified": metrics.get("local_inference_verified"),
    "gpu_activity_observed": metrics.get("gpu_activity_observed"),
}}
Path({summary_path.as_posix()!r}).write_text(json.dumps(summary, indent=2) + "\\n")
PY
if [ -n "$(git status --porcelain)" ]; then
  git add -A
  git commit -m "Mixmod solution"
else
  git commit --allow-empty -m "Mixmod empty solution"
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
    mixmod_run = (
        "set -euo pipefail; "
        f"{PATH_SETUP} "
        f"{mixmod_install_command or _default_mixmod_install(mixmod_command)}; "
        f"{shlex.quote(mixmod_command)} --version; "
        "codex --version; "
        "opencode --version"
    )
    return AgentInstallSpec(
        agent_name=agent_name,
        version=version,
        cache_key="mixmod-deepswe-agent-v5",
        steps=[
            InstallStep(user="root", env={"DEBIAN_FRONTEND": "noninteractive"}, run=root_run),
            InstallStep(user="agent", run=node_run),
            InstallStep(user="agent", run=mixmod_run),
        ],
        verification_command=f"{PATH_SETUP}{shlex.quote(mixmod_command)} --version",
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
