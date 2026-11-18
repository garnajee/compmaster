#!/usr/bin/env bash
set -euo pipefail

command -v docker >/dev/null || { echo 'Docker CLI est requis.' >&2; exit 1; }
bash "$(dirname "${BASH_SOURCE[0]}")/compose.sh" version >/dev/null || { echo 'Le plugin Docker Compose est requis.' >&2; exit 1; }
docker info >/dev/null || { echo 'Le moteur Docker doit être démarré.' >&2; exit 1; }
task_root="${1:-}"
if [[ -z "$task_root" ]]; then
  read -r -p 'Dossier racine des projets Compose (chemin absolu) : ' task_root
fi
[[ -d "$task_root" ]] || { echo 'Dossier introuvable.' >&2; exit 1; }
task_root="$(cd "$task_root" && pwd -P)"
[[ "$task_root" != / ]] || { echo 'Choisissez un dossier de projets, pas la racine du système.' >&2; exit 1; }
[[ "$task_root" != *"'"* && "$task_root" != *$'\n'* ]] || { echo 'Chemin contenant une apostrophe ou un saut de ligne non supporté.' >&2; exit 1; }
task_endpoint="${DOCKER_HOST:-$(docker context inspect --format '{{.Endpoints.docker.Host}}')}"
[[ "$task_endpoint" == unix://* ]] || { echo 'Cette installation nécessite un socket Unix local (Linux, macOS ou WSL2).' >&2; exit 1; }
task_socket="${task_endpoint#unix://}"
[[ -S "$task_socket" ]] || { echo "Socket introuvable : $task_socket" >&2; exit 1; }
task_gid="$(stat -c '%g' "$task_socket" 2>/dev/null || stat -f '%g' "$task_socket")"
task_uid="$(id -u)"
task_user_gid="$(id -g)"
[[ "$task_uid" != 0 ]] || { echo 'Exécutez la configuration avec votre compte non-root (sans sudo).' >&2; exit 1; }
task_port=8080
task_published="$(docker ps --format '{{.Ports}}')"
while printf '%s\n' "$task_published" | grep -Eq ":${task_port}->" || (: </dev/tcp/127.0.0.1/"$task_port") 2>/dev/null; do
  task_port=$((task_port + 1))
  [[ "$task_port" -lt 8280 ]] || { echo 'Aucun port disponible entre 8080 et 8279. Configurez COMPMASTER_PORT manuellement.' >&2; exit 1; }
done
# Desktop exposes its VM socket with different ownership. Inspect that socket via an ephemeral container.
# This only reads the socket metadata, never connects to the daemon from that container.
if [[ "$(uname -s)" == Darwin ]]; then
  if [[ "$task_endpoint" == *'.colima/'* ]]; then
    task_socket=/var/run/docker.sock
  else
    task_socket=/var/run/docker.sock.raw
  fi
  task_gid="$(docker run --rm --network none --read-only --cap-drop ALL --security-opt no-new-privileges -v "$task_socket:/socket:ro" alpine:3.23 stat -c '%g' /socket)"
fi
task_repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
task_env="$task_repo/.env"
if [[ -e "$task_env" ]]; then
  echo "Le fichier $task_env existe déjà. Il n'a pas été écrasé. Modifiez-le ou déplacez-le pour relancer la configuration." >&2
  exit 1
fi
umask 077
{
  printf "COMPOSE_ROOT='%s'\n" "$task_root"
  printf "DOCKER_SOCKET='%s'\n" "$task_socket"
  printf 'DOCKER_GID=%s\n' "$task_gid"
  printf 'COMPMASTER_PORT=%s\n' "$task_port"
  printf 'COMPMASTER_UID=%s\n' "$task_uid"
  printf 'COMPMASTER_GID=%s\n' "$task_user_gid"
} > "$task_env"
printf '\nConfiguration créée : %s\nLancez : bash scripts/compose.sh pull && bash scripts/compose.sh up -d\nPuis : http://127.0.0.1:%s\n' "$task_env" "$task_port"
echo 'Le dossier sera monté en lecture-écriture. Les chemins extérieurs à cette racine nécessaires aux projets doivent être montés explicitement.'
