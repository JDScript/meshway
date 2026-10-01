#!/bin/sh
# Run in an owned tmux. The runner acquires/reuses the common performance lock.
set -eu
umask 077
: "${ART:?New private approved artifact directory required}"
: "${IMAGE:?Frozen resolved image ID required}"
: "${ARM:?Unique arm name required}"
: "${MESHWAY_PERF_LOCK:?Shared performance lock required}"
for run in 1 2; do
  python3 experiments/tunnel-throughput/managed.py --image "$IMAGE" \
    --seconds 20 --iterations 1 --parallel 1 4 --directions forward reverse \
    --loaded-rtt --require-runtime-statistics \
    --output "$ART/$ARM-r$run.json" --temp-dir "$ART" > "$ART/$ARM-r$run.log" 2>&1
done
