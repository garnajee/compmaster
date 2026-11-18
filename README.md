# CompMaster

[![Publish container image](https://github.com/garnajee/compmaster/actions/workflows/publish.yml/badge.svg)](https://github.com/garnajee/compmaster/actions/workflows/publish.yml)

A local Docker control panel that discovers your Compose projects automatically, regardless of their file names or directory layout.

CompMaster combines a Rust/Axum backend, a responsive React interface and SQLite persistence in a **distroless, non-root container**. Install the prebuilt image from **`ghcr.io/garnajee/compmaster`**: no Rust, Node.js or local image build is needed.

## Table of contents

- [Features](#features)
- [Supported architectures](#supported-architectures)
- [Quick start](#quick-start)
- [Docker Compose](#docker-compose)
- [Configuration](#configuration)
- [Storage and permissions](#storage-and-permissions)
- [Updates and version pinning](#updates-and-version-pinning)
- [Project discovery](#project-discovery)
- [Ports and conflict detection](#ports-and-conflict-detection)
- [Private registries](#private-registries)
- [Security](#security)
- [GHCR publishing workflow](#ghcr-publishing-workflow)
- [Build from source](#build-from-source)
- [Development and testing](#development-and-testing)
- [Documentation](#documentation)
- [License](#license)

## Features

- Recursive discovery of `.yml` and `.yaml` Compose files with arbitrary names, including multiple projects in one directory.
- Existing Compose deployments grouped by Docker labels; standalone containers listed separately.
- Search, status and health filters, favorites, CPU/memory snapshots and live Docker events.
- Service details, network addresses, published ports, mounts, images and diagnostics.
- Compose pull, build, up, start, stop, restart and down, with persistent job history.
- Live logs with filtering/export and a terminal inside running containers.
- YAML editing with syntax and Compose validation, revision checks, atomic saves and backups.
- Controlled container replacement with a retained backup and rollback on failure.
- Image, network and volume management, plus previewed, explicitly selected cleanup.

## Supported architectures

| Architecture | Platform | Availability |
|---|---|---|
| x86-64 | `linux/amd64` | Prebuilt GHCR image |
| ARM64 / AArch64 | `linux/arm64` | Prebuilt GHCR image |

Both architectures share the same tags. Docker automatically pulls the correct platform; users do not need an architecture-specific Compose file. Other CPU architectures and native Windows containers are not currently provided.

CompMaster works with a local Unix Docker socket on Linux, Docker Desktop or Colima on macOS, and a consistent Linux Docker environment inside WSL2. With Docker Desktop/WSL2, verify that the daemon sees bind-mount paths at the same Linux paths as CompMaster; WSL integration alone does not guarantee this.

## Quick start

Prerequisites: a running Docker engine, Docker CLI, Docker Compose v2 or newer, and a non-root account with access to Docker.

```sh
git clone https://github.com/garnajee/compmaster.git
cd compmaster

# Select the directory containing your projects.
bash scripts/setup.sh /absolute/path/to/your/projects

# Download and start the prebuilt image. Nothing is compiled locally.
bash scripts/compose.sh pull
bash scripts/compose.sh up -d
```

Open **http://127.0.0.1:8080**, or the available port printed by setup. The script detects the active local Docker socket and its group, uses your non-root UID/GID, selects an available web port starting at 8080, and creates a private `.env`. It refuses to overwrite an existing `.env`.

`scripts/compose.sh` also finds Compose in common Homebrew and Docker Desktop locations without changing global Docker configuration. If `docker compose` already works, you can use it directly.

**First publication:** GHCR initially creates new packages as private. The maintainer must make `compmaster` public in its package settings before other users can pull anonymously. Until the first successful publication, `latest` is not available. See [GHCR publishing workflow](#ghcr-publishing-workflow).

## Docker Compose

[compose.yaml](compose.yaml) uses GHCR by default. Alternatively, create a deployment directory with this Compose file and configure `.env` using [.env.example](.env.example):

```yaml
name: compmaster
services:
  compmaster:
    image: ${COMPMASTER_IMAGE:-ghcr.io/garnajee/compmaster:latest}
    restart: unless-stopped
    user: "${COMPMASTER_UID:-65532}:${COMPMASTER_GID:-65532}"
    group_add:
      - "${DOCKER_GID}"
      - "65532"
    ports:
      - "127.0.0.1:${COMPMASTER_PORT:-8080}:8080"
    environment:
      COMPOSE_ROOT: "${COMPOSE_ROOT}"
      COMPMASTER_PUBLIC_ORIGIN: "http://127.0.0.1:${COMPMASTER_PORT:-8080}"
      COMPMASTER_PROBE_HOST: host.docker.internal
    volumes:
      - "${COMPOSE_ROOT}:${COMPOSE_ROOT}"
      - "${DOCKER_SOCKET}:/var/run/docker.sock"
      - compmaster-data:/data
    extra_hosts:
      - "host.docker.internal:host-gateway"
    read_only: true
    tmpfs:
      - /tmp:rw,noexec,nosuid,size=128m,mode=1777
    cap_drop: [ALL]
    security_opt:
      - no-new-privileges:true
    pids_limit: 256
    stop_grace_period: 20s
volumes:
  compmaster-data:
```

The repository's Compose file uses long-form bind mounts with `create_host_path: false` to reject missing source paths. When copying the example above, ensure the root directory and socket already exist.

## Configuration

Deployment variables are read from `.env`; setup fills in the host-specific values.

| Variable | Purpose | Default |
|---|---|---|
| `COMPOSE_ROOT` | Absolute project root, mounted at the same path | Required |
| `DOCKER_SOCKET` | Socket mounted into CompMaster | Detected by setup |
| `DOCKER_GID` | Numeric group allowed to access the socket | Detected by setup |
| `COMPMASTER_UID` | Non-root runtime UID | `65532`; setup uses your UID |
| `COMPMASTER_GID` | Runtime primary GID | `65532`; setup uses your GID |
| `COMPMASTER_PORT` | Local web port | `8080`, or the next available port |
| `COMPMASTER_IMAGE` | Image tag or digest to deploy | `ghcr.io/garnajee/compmaster:latest` |
| `DOCKER_CONFIG_DIR` | Optional registry credentials directory | Unset |

Advanced settings include `COMPMASTER_LISTEN`, `COMPMASTER_PUBLIC_ORIGIN`, `COMPMASTER_DATA_DIR`, `COMPMASTER_STATIC_DIR`, `COMPMASTER_PROBE_HOST`, `COMPMASTER_SCAN_CONCURRENCY`, `DOCKER_HOST`, `DOCKER_CLI`, `DOCKER_CONFIG` and `RUST_LOG`. The default Compose file already configures the container's listen address, data directory and socket.

## Storage and permissions

- `/data` stores SQLite state, preferences, job history and YAML backups in the named data volume.
- The project root is mounted **read/write at exactly the same absolute path**, preserving relative build contexts, `.env`, `env_file` and bind mounts.
- Paths outside this root that a project needs must be mounted explicitly at their original locations.
- The image defaults to UID/GID 65532. Setup uses your non-root host UID/GID to preserve file ownership during atomic saves. Supplementary group 65532 grants access to the data volume.
- A read/write mount does not override filesystem permissions. CompMaster does not recursively change ownership or permissions on your projects.

Back up the data volume before upgrades. `compose down` retains it; adding `-v` deletes it. Container replacement preserves existing volumes, but does not back up or roll back application data or database migrations. Old containers remain under `*-cm-backup-*` names until explicitly deleted.

## Updates and version pinning

```sh
bash scripts/compose.sh pull
bash scripts/compose.sh up -d
```

| Tag | Meaning |
|---|---|
| `latest` | Most recent successfully validated build of `main` |
| `sha-<full-commit-sha>` | Build associated with a source commit |
| `X.Y.Z`, `vX.Y.Z` | Version published from the corresponding Git tag |
| `X.Y` | Latest validated patch published for that release line |
| `X` | Major release alias, for major versions greater than zero |

Release tags do not move the rolling `latest` tag away from `main`. Pre-release versions keep their pre-release identifiers. Version tags only exist after their Git tags have been pushed and the publishing workflow has succeeded.

Select a published version in `.env`:

```dotenv
COMPMASTER_IMAGE=ghcr.io/garnajee/compmaster:0.1.0
```

For exact reproducibility, pin the image's multi-platform digest instead of a mutable tag:

```dotenv
COMPMASTER_IMAGE=ghcr.io/garnajee/compmaster@sha256:<published-manifest-digest>
```

## Project discovery

CompMaster recursively inspects YAML files without following symlinks. Compose candidates are validated with `docker compose config`; unrelated valid YAML is ignored and malformed candidates remain visible with diagnostics. `.compmasterignore` uses `.gitignore`-style patterns.

Each independent Compose file becomes a project. Without an explicit `name`, a stable `cm-<directory>-<file>-<hash8>` name avoids collisions. Ambiguous project names block Compose actions.

The `com.docker.compose.project` and `com.docker.compose.service` labels group existing containers without changing them. Original paths are hints: files must be accessible and their combined configuration and services must match before file-based operations are enabled. Verified multi-file deployments preserve their `-f` ordering. Otherwise, runtime inspection, logs and terminal access remain available.

YAML/environment-file changes, Docker events and a 30-second reconciliation refresh the inventory. Editing validates the syntax and combined Compose model, checks the previous revision, backs up the original and atomically replaces the file.

## Ports and conflict detection

The Ports page combines published ports, stopped-container reservations, Compose declarations and best-effort TCP probes. Protocols and listen addresses remain visible; wildcard IPv4/IPv6 bindings are handled conservatively.

Known conflicts block operations that start containers. Stopped projects and undeployed YAML files are potential reservations, not active listeners. Alternative ports are suggestions: changes require reviewing a YAML diff and confirming a validated save.

A TCP probe connects without sending data. It cannot identify the host process or guarantee availability on every interface, especially localhost, IPv6 and UDP. Timeouts and unreachable hosts remain unknown. Docker can still report a conflict at bind time. Reconciliation probes at most 100 distinct requested TCP ports; use the manual probe for others.

## Private registries

To pull private application images, provide a dedicated Docker `config.json` readable by the runtime user. Add this to `.env`:

```dotenv
DOCKER_CONFIG_DIR='/absolute/path/to/docker-config'
```

Enable the read-only credentials mount:

```sh
bash scripts/compose.sh -f compose.yaml -f compose.registry.yaml up -d
```

Inline `auths` entries and explicitly mounted, compatible Linux credential helpers are supported. Native `desktop`, `osxkeychain` and `wincred` helpers cannot run in this Linux container and produce a diagnostic. Credentials are not imported into SQLite.

Registry digest checks are cached for six hours. Local images and unavailable registries are reported accordingly. Update checks do not automatically pull or redeploy images.

## Security

CompMaster has **no authentication** and is intended for a trusted local user. Keep its published port bound to `127.0.0.1`; do not expose it directly to the LAN or Internet.

The Docker socket grants administrative control over the engine and potentially the host, even when CompMaster is non-root. The runtime uses a read-only root filesystem, dropped capabilities, `no-new-privileges`, Host/Origin validation, CSRF protection and no CORS.

Compose and Buildx are rebuilt from official tags with patched dependencies using a pinned Go toolchain. Every published architecture passes image execution tests and the blocking Trivy gate. See [security details](docs/SECURITY.md) for the process and scan history.

## GHCR publishing workflow

[publish.yml](.github/workflows/publish.yml) runs on pushes to `main`, version tags matching `vX.Y.Z`, and manual dispatches. It calls the reusable [CI workflow](.github/workflows/ci.yml), which:

1. Runs Rust, frontend, browser and Docker API tests.
2. Builds amd64 and arm64 images on native GitHub-hosted runners.
3. Exercises each image, including build/up/down and non-root permissions.
4. Runs Trivy and exports validated images as short-lived artifacts.

Only after **all checks succeed** does the publishing job log in to GHCR, push those exact images and assemble their multi-platform manifest. It does not rebuild images after scanning. Pull requests only run validation and cannot publish packages. `GITHUB_TOKEN` receives `packages: write` only in the publishing job; no personal access token is needed as a repository secret.

For the first publication, open [the package page](https://github.com/users/garnajee/packages/container/package/compmaster), then **Package settings → Change visibility → Public** to enable anonymous pulls. GitHub initially creates personal-account packages as private; repository visibility alone does not make a GHCR package public.

To publish a release after updating the application version:

```sh
git tag v0.1.0
git push origin v0.1.0
```

For a fork, the workflow derives the lowercase registry path from `github.repository`. Set `COMPMASTER_IMAGE` to your fork's image when deploying.

## Build from source

Building is optional. Use the explicit source-build override:

```sh
bash scripts/setup.sh /absolute/path/to/your/projects
bash scripts/compose.sh -f compose.yaml -f compose.build.yaml up -d --build
```

Or build directly:

```sh
docker build -t compmaster:local .
docker buildx build --platform linux/amd64,linux/arm64 .
```

## Development and testing

Development requires a recent stable Rust toolchain, Node.js 24+, and Docker/Compose for integration tests.

```sh
npm --prefix frontend ci
npm --prefix frontend run build
COMPOSE_ROOT=/absolute/path/to/projects \
  DOCKER_HOST=unix:///path/to/docker.sock cargo run
```

The backend serves `frontend/dist` at `http://127.0.0.1:8080`. For Vite, run it with `COMPMASTER_PUBLIC_ORIGIN=http://127.0.0.1:5173`, then `npm --prefix frontend run dev -- --host 127.0.0.1` in another terminal.

The development binary does not infer host Docker contexts: set `DOCKER_HOST` from `docker context inspect --format '{{.Endpoints.docker.Host}}'` when needed. Configure `cliPluginsExtraDirs`, or point `DOCKER_CLI` at a wrapper, if your CLI cannot find Compose.

```sh
cargo test
npm --prefix frontend test
npm --prefix frontend run build
npm --prefix frontend run test:e2e
```

Real-engine tests require an explicit opt-in:

```sh
export DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
export COMPMASTER_DOCKER_TESTS=1
cargo test -- --ignored
cargo build
node scripts/test-setup.mjs
node scripts/test-api.mjs
node scripts/test-image.mjs compmaster:local
```

Install Chromium for browser tests with `cd frontend && npx playwright install chromium`. Docker tests create and clean their own `cm-test-*` resources. Use a development engine, not production infrastructure, for replacement and deletion tests.

## Documentation

- [API reference](docs/API.md)
- [Validation and platform limitations](docs/VALIDATION.md)
- [Security and patched Docker plugins](docs/SECURITY.md)
- [Deployment Compose file](compose.yaml)
- [Environment example](.env.example)

The interface is currently in French; this README is the English installation and operations guide.

## License

GNU General Public License v3. See [LICENSE](LICENSE). Provided without warranty.
