#!/usr/bin/env bash
# Reproduce the R0 spike numbers. Usage: run.sh <spike-root> <results-label> [--no-server]
# <spike-root> holds data/ and extra/ (see docs/v2/rule-sets-design.md#spike-实测).
set -euo pipefail
root=$(cd "$1" && pwd)
label=$2
features=()
[[ "${3:-}" == "--no-server" ]] && features=(--no-default-features)
here=$(cd "$(dirname "$0")" && pwd)
export CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$here/target"
cargo build --release --locked ${features[@]+"${features[@]}"} --manifest-path "$here/Cargo.toml"
bin="$CARGO_TARGET_DIR/release/meshway-rules-spike"
out="$root/results/$label"
mkdir -p "$out"
{
  uname -a
  if [[ -r /proc/cpuinfo ]]; then grep -m1 "model name" /proc/cpuinfo; else sysctl -n machdep.cpu.brand_string; fi
  date -u +%FT%TZ
  git -C "$here" rev-parse HEAD
  rustc --version
} > "$out/host.txt"
"$bin" coverage "$root" > "$out/coverage.jsonl"
cp "$root/out/coverage.json" "$out/coverage.json"
sets=(sing-geosite-cn mihomo-geosite-cn 'sing-geosite-geolocation-!cn' sing-geoip-cn mihomo-geoip-cn bm7-global bm7-advertising)
for set in "${sets[@]}"; do
  for run in 1 2 3; do
    "$bin" bench "$root" "$set" 1000000 > "$out/bench-$set-$run.txt"
  done
done
for ext in mwrs mwrh; do
  for mode in heap mmap shared; do
    "$bin" memory "$root" mihomo-geosite-cn 1000 "$mode" "$ext" >> "$out/memory.txt"
    "$bin" memory "$root" sing-geosite-cn 1000 "$mode" "$ext" >> "$out/memory.txt"
  done
done
for f in "$root"/out/*.mwrs "$root"/out/*.mwrh; do
  printf '%s raw=%s gzip9=%s\n' "$(basename "$f")" "$(wc -c < "$f" | tr -d ' ')" "$(gzip -9 -c "$f" | wc -c | tr -d ' ')"
done > "$out/sizes.txt"
echo "results in $out"
