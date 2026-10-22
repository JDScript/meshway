#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
if [[ "${MESHWAY_REUSE_CORE:-0}" == 1 ]]; then
  # Swift-only iteration: the caller supplies a matching core and its bindings.
  test -d "$root/build/MeshwayCore.xcframework"
  test -f "$root/build/bindings/meshway_apple_bridge.swift"
else
  bash "$root/scripts/build-core.sh"
fi
exec xcodebuild -project "$root/Meshway.xcodeproj" -scheme Meshway \
  -configuration "${CONFIGURATION:-Debug}" -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath "$root/build/DerivedData" "$@" build
