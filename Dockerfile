# syntax=docker/dockerfile:1.7
FROM node:24-alpine AS frontend
WORKDIR /build/frontend
COPY frontend/package*.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

FROM rust:1.98-alpine AS backend
ARG TARGETARCH
RUN apk add --no-cache musl-dev sqlite-dev sqlite-static pkgconf
ENV LIBSQLITE3_SYS_USE_PKG_CONFIG=1 SQLITE3_STATIC=1
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=compmaster-target-${TARGETARCH},target=/build/target,sharing=locked \
    cargo build --release --locked && cp /build/target/release/compmaster /build/compmaster
RUN mkdir -p /runtime/data/backups && chown -R 65532:65532 /runtime/data \
    && chmod 2770 /runtime/data /runtime/data/backups

FROM docker:29.9.3-cli@sha256:5a7af0dcb0e46c803efe9b45371dd21cb500e5b4453250394ac6f06d49b6e6f1 AS docker-cli

FROM gcr.io/distroless/static-debian13:nonroot@sha256:aafe993665bf6200db38a02c11a9351e782a84b6a45e7f33b6695638c88e3931
LABEL org.opencontainers.image.title="CompMaster" \
      org.opencontainers.image.description="Local Docker and Compose control panel" \
      org.opencontainers.image.licenses="GPL-3.0-only"
COPY --from=backend /build/compmaster /usr/local/bin/compmaster
COPY --from=backend --chown=65532:65532 /runtime/ /
COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/docker-compose /usr/local/libexec/docker/cli-plugins/docker-compose
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/docker-buildx /usr/local/libexec/docker/cli-plugins/docker-buildx
COPY --from=frontend /build/frontend/dist /app/public
COPY LICENSE /app/LICENSE
ENV COMPMASTER_LISTEN=0.0.0.0:8080 \
    COMPMASTER_DATA_DIR=/data \
    COMPMASTER_STATIC_DIR=/app/public \
    DOCKER_CONFIG=/docker-config \
    BUILDX_CONFIG=/tmp/buildx \
    DOCKER_HOST=unix:///var/run/docker.sock
USER 65532:65532
WORKDIR /app
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s CMD ["/usr/local/bin/compmaster", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/compmaster"]
