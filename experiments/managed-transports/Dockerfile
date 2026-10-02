# syntax=docker/dockerfile:1
# Build from the workspace root; credentials are created only in runtime tmpfs.
FROM rust:1.98-slim-trixie@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7 AS builder
ARG TARGETARCH
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential cmake pkg-config libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
ENV CARGO_BUILD_JOBS=2
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/agent ./apps/agent
COPY apps/controller ./apps/controller
COPY apps/apple/core ./apps/apple/core
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target,id=meshway-managed-${TARGETARCH},sharing=locked \
    cargo build --locked --release -p meshway-controller -p meshway-agent --bin meshway-controller --bin meshway-agent --features meshway-agent/managed-connections \
    && install -D target/release/meshway-agent /out/meshway-agent \
    && install -D target/release/meshway-controller /out/meshway-controller

FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates curl iproute2 iputils-ping python3 util-linux \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /out/meshway-agent /out/meshway-controller /usr/local/bin/
RUN ln -s /usr/local/bin/meshway-agent /usr/local/bin/meshway
ENV MESHWAY_STATE=/state/agent.json
COPY experiments/managed-connections/container.py /opt/meshway-managed/container.py
ENTRYPOINT ["sleep", "infinity"]
