#!/bin/sh
# Invoke through experiments/transport/performance_lock.py; never nest locks.
set -eu
rustfmt --edition 2024 apps/agent/src/bin/carrier_measure.rs \
  apps/agent/src/managed_dataplane/throughput.rs apps/agent/src/managed_dataplane/packet.rs \
  crates/dataplane/src/runtime/tests/discovery.rs
cargo fmt --all --check
python3 -m unittest discover -s experiments/tunnel-throughput -p 'test_*.py'
# Socket-using Rust behavior tests stay in Linux build namespaces, not the host.
docker build --platform linux/arm64 --target verification \
  -f experiments/tunnel-throughput/Dockerfile.managed -t meshway-throughput:verified-e69cb3d .
