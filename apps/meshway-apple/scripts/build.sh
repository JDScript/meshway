#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
bash "$root/scripts/build-core.sh"
exec xcodebuild -project "$root/Meshway.xcodeproj" -scheme Meshway \
  -configuration "${CONFIGURATION:-Debug}" -destination 'platform=macOS,arch=arm64' \
  -derivedDataPath "$root/build/DerivedData" "$@" build
