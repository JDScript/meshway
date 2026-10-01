# syntax=docker/dockerfile:1
FROM rust:1.98-slim-trixie@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7 AS builder
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends build-essential cmake pkg-config libsqlite3-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/agent ./apps/agent
COPY apps/controller ./apps/controller
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target,id=meshway-lab-${TARGETARCH},sharing=locked \
    cargo build --locked --release -p meshway-agent --features transport-lab \
    && install -D target/release/meshway-agent /out/meshway-agent \
    && install -D target/release/meshway-carrier-measure /out/meshway-carrier-measure
FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates iperf3 iproute2 iputils-ping python3 ethtool
COPY --from=builder /out/meshway-agent /usr/local/bin/meshway-agent
COPY --from=builder /out/meshway-carrier-measure /usr/local/bin/meshway-carrier-measure
ENTRYPOINT ["meshway-agent"]
