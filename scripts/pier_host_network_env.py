"""Pier Docker environment override for host-network local model runs."""

from __future__ import annotations

import json
from pathlib import Path

from pier.environments.docker.docker import DockerEnvironment


class HostNetworkDockerEnvironment(DockerEnvironment):
    """Run the Pier main container on the host network.

    This is useful when an agent inside Pier must reach a host-local model
    server, such as llama-server on 127.0.0.1.
    """

    def _prepare_egress_proxy_compose(self) -> None:
        self._egress_proxy_compose_path = None
        self._egress_proxy_env = {}

    @property
    def _docker_compose_paths(self) -> list[Path]:
        build_or_prebuilt = (
            self._DOCKER_COMPOSE_PREBUILT_PATH
            if self._use_prebuilt
            else self._DOCKER_COMPOSE_BUILD_PATH
        )

        paths = [self._DOCKER_COMPOSE_BASE_PATH]
        if self._resources_compose_path:
            paths.append(self._resources_compose_path)
        paths.append(build_or_prebuilt)

        if self._is_windows_container:
            paths.append(self._DOCKER_COMPOSE_WINDOWS_KEEPALIVE_PATH)

        if self._environment_docker_compose_path.exists():
            paths.append(self._environment_docker_compose_path)

        if self._mounts_compose_path:
            paths.append(self._mounts_compose_path)

        host_network_path = self.trial_paths.trial_dir / "docker-compose-host-network.json"
        host_network_path.write_text(
            json.dumps({"services": {"main": {"network_mode": "host"}}}, indent=2)
            + "\n"
        )
        paths.append(host_network_path)
        return paths
