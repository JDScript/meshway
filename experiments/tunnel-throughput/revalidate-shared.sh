#!/bin/sh
# Run in an owned tmux session. Build/checks and each runner use the shared lock.
set -eu
: "${ART:?Existing new approved opencode TMP directory required}"
: "${MESHWAY_PERF_LOCK:?Shared performance lock required}"
IMAGE=meshway-throughput:shared-pump-4edb70e
python3 experiments/transport/performance_lock.py sh -c \
  "cargo fmt --all --check && python3 -m unittest discover -s experiments/tunnel-throughput -p 'test_*.py' && docker build --platform linux/arm64 --target verification -f experiments/tunnel-throughput/Dockerfile.managed -t meshway-throughput:shared-pump-verified-4edb70e . && docker build --platform linux/arm64 -f experiments/tunnel-throughput/Dockerfile.managed -t $IMAGE . && docker image inspect --format '{{.Id}}' $IMAGE > '$ART/image-id.txt'"
IMAGE=$(cat "$ART/image-id.txt")
# ABBA order balances time/order bias. New fixtures/keys in each arm; the same
# resolved binary/image is used throughout, both paths and both directions P4.
for arm in q32-r1 q128-r1 q128-r2 q32-r2; do
  case "$arm" in q32-*) queue=32 ;; q128-*) queue=128 ;; esac
  python3 experiments/tunnel-throughput/managed.py --image "$IMAGE" \
    --seconds 20 --iterations 1 --parallel 4 --directions forward reverse \
    --tun-queue-length "$queue" --loaded-rtt --require-runtime-statistics \
    --output "$ART/$arm.json" --temp-dir "$ART" > "$ART/$arm.log" 2>&1
done
python3 experiments/transport/performance_lock.py python3 experiments/tunnel-throughput/summarize.py \
  "$ART/q32-r1.json" "$ART/q128-r1.json" "$ART/q128-r2.json" "$ART/q32-r2.json" > "$ART/summary.txt"
