#!/bin/sh
set -eu
: "${ART:?Set ART to existing approved opencode TMP}"
: "${MESHWAY_PERF_LOCK:?Set shared performance lock}"
python3 experiments/tunnel-throughput/runtime.py --image meshway-throughput:runtime-fixed-e69cb3d \
  --output "$ART/runtime10-final.json" --temp-dir "$ART" > "$ART/runtime10-final.log" 2>&1
python3 experiments/tunnel-throughput/run.py --image meshway-throughput:e69cb3d --modes bare --bare-mtus 1000 \
  --output "$ART/bare1000.json" --temp-dir "$ART" > "$ART/bare1000.log" 2>&1
python3 experiments/transport/diagnose.py --image meshway-throughput:e69cb3d --workers 2 --work-budget 16 \
  --queue 64 --udp-receive-buffer 0 --omit 2 --seconds 20 --iterations 2 \
  --output "$ART/queue64-buffer0.json" --temp-dir "$ART" > "$ART/queue64-buffer0.log" 2>&1
python3 experiments/transport/diagnose.py --image meshway-throughput:e69cb3d --workers 2 --work-budget 16 \
  --queue 256 --udp-receive-buffer 1048576 --omit 2 --seconds 20 --iterations 2 \
  --output "$ART/queue256-buffer1m.json" --temp-dir "$ART" > "$ART/queue256-buffer1m.log" 2>&1
python3 experiments/transport/performance_lock.py sh experiments/tunnel-throughput/verify.sh
python3 experiments/transport/performance_lock.py docker build --platform linux/arm64 \
  -f experiments/tunnel-throughput/Dockerfile.managed -t meshway-throughput:managed-tun128-e69cb3d .
python3 experiments/tunnel-throughput/managed.py --image meshway-throughput:managed-tun128-e69cb3d \
  --require-runtime-statistics --output "$ART/managed10-default128.json" --temp-dir "$ART" > "$ART/managed10-default128.log" 2>&1
python3 experiments/transport/performance_lock.py python3 experiments/tunnel-throughput/summarize.py \
  "$ART/matrix10-fixed.json" "$ART/managed20-base.json" "$ART/managed20-budget16.json" "$ART/managed20-tun128.json" \
  "$ART/runtime10-final.json" "$ART/bare1000.json" "$ART/queue64-buffer0.json" "$ART/queue256-buffer1m.json" "$ART/managed10-default128.json" \
  > "$ART/summary.txt"
