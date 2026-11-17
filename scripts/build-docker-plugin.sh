#!/bin/sh
# Runs only in the Go build stage; no Go toolchain is copied into the runtime image.
set -eu
plugin="${1:?plugin required}"
target_arch="${2:?target architecture required}"
case "$target_arch" in amd64|arm64) ;; *) echo "Unsupported architecture: $target_arch" >&2; exit 1 ;; esac
case "$plugin" in
  buildx)
    module=github.com/docker/buildx
    upstream=v0.39.0
    dependency=github.com/moby/go-archive
    fixed=v0.3.3
    package=./cmd/buildx
    version_symbol=github.com/docker/buildx/version.Version
    ;;
  compose)
    module=github.com/docker/compose/v5
    upstream=v5.7.2
    dependency=golang.org/x/net
    fixed=v0.62.0
    package=./cmd
    version_symbol=github.com/docker/compose/v5/internal.Version
    ;;
  *) echo "Unsupported plugin: $plugin" >&2; exit 1 ;;
esac

export GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org
go mod download "$module@$upstream"
mkdir -p "/src/$plugin" /out
cp -R "$(go env GOPATH)/pkg/mod/$module@$upstream/." "/src/$plugin/"
chmod -R u+w "/src/$plugin"
cd "/src/$plugin"
go mod edit "-require=$dependency@$fixed"
go mod download "$dependency"
GOARCH="$target_arch" go build -mod=mod -trimpath \
  -ldflags "-s -w -X $version_symbol=$upstream-compmaster.1" \
  -o "/out/docker-$plugin" "$package"
# Fail the build if module resolution ever stops selecting the patched dependency.
go version -m "/out/docker-$plugin" | grep -F "$dependency" | grep -F "$fixed"
