#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
for plist in "$root"/Config/*.plist "$root"/Config/*.entitlements; do
  plutil -lint "$plist"
done
for script in "$root"/scripts/*.sh; do
  bash -n "$script"
done
exec bash "$root/scripts/build.sh" CODE_SIGNING_ALLOWED=NO
