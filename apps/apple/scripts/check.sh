#!/bin/bash
set -euo pipefail
APPLE="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$APPLE/../.." && pwd)"
cd "$ROOT"
cargo fmt --check
cargo clippy --locked -p meshway-apple-core --all-targets --features bindgen -- -D warnings
cargo check --locked --workspace
bash "$APPLE/scripts/build-core.sh"
swift test --package-path "$APPLE/MeshwayKit"
for destination in 'platform=macOS,arch=arm64' 'generic/platform=iOS Simulator'; do
  xcodebuild -project "$APPLE/Meshway.xcodeproj" -scheme Meshway \
    -configuration Debug -destination "$destination" \
    -derivedDataPath "$APPLE/build/DerivedData" CODE_SIGNING_ALLOWED=NO ARCHS=arm64 build
done
