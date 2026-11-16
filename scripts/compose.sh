#!/usr/bin/env bash
# Locate Compose without changing the user's global Docker configuration.
set -euo pipefail
task_repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$task_repo"
if docker compose version >/dev/null 2>&1; then
  exec docker compose "$@"
fi
for task_plugin in \
  "${DOCKER_CONFIG:-$HOME/.docker}/cli-plugins/docker-compose" \
  /opt/homebrew/lib/docker/cli-plugins/docker-compose \
  /opt/homebrew/libexec/docker/cli-plugins/docker-compose \
  /usr/local/lib/docker/cli-plugins/docker-compose \
  /usr/local/libexec/docker/cli-plugins/docker-compose \
  /usr/lib/docker/cli-plugins/docker-compose \
  /usr/libexec/docker/cli-plugins/docker-compose \
  /Applications/Docker.app/Contents/Resources/cli-plugins/docker-compose; do
  if [[ -x "$task_plugin" ]] && "$task_plugin" version >/dev/null 2>&1; then
    exec "$task_plugin" "$@"
  fi
done
if command -v docker-compose >/dev/null; then
  task_version="$(docker-compose version)"
  if [[ "$task_version" != *'version 1.'* && "$task_version" != *'version v1.'* ]]; then
    exec docker-compose "$@"
  fi
fi
echo 'Docker Compose v2 ou plus récent est requis. Installez le plugin Compose pour votre Docker CLI.' >&2
exit 1
