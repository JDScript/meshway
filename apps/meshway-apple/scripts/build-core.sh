#!/bin/bash
set -euo pipefail
apple="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$apple/../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
export CARGO_INCREMENTAL=0
export MACOSX_DEPLOYMENT_TARGET=26.0
profile="${MESHWAY_CORE_PROFILE:-release}"
case "$profile" in
  release) cargo_profile=release ;;
  debug) cargo_profile=dev ;;
  *) echo "MESHWAY_CORE_PROFILE must be debug or release" >&2; exit 2 ;;
esac
cd "$root"
mise exec -- env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" CARGO_INCREMENTAL=0 MACOSX_DEPLOYMENT_TARGET=26.0 cargo build --locked -p meshway-apple-bridge --profile "$cargo_profile" --target aarch64-apple-darwin
mkdir -p "$apple/build/bindings" "$apple/build/headers"
mise exec -- env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" CARGO_INCREMENTAL=0 cargo run --locked -p meshway-apple-bridge --features bindgen --bin meshway-apple-bindgen -- \
  generate --library "$CARGO_TARGET_DIR/aarch64-apple-darwin/$profile/libmeshway_apple_bridge.dylib" \
  --language swift --out-dir "$apple/build/bindings"
cp "$apple/build/bindings/meshway_apple_bridgeFFI.h" "$apple/build/headers/"
cp "$apple/build/bindings/meshway_apple_bridgeFFI.modulemap" "$apple/build/headers/module.modulemap"
rm -rf "$apple/build/MeshwayCore.xcframework"
xcodebuild -create-xcframework \
  -library "$CARGO_TARGET_DIR/aarch64-apple-darwin/$profile/libmeshway_apple_bridge.a" \
  -headers "$apple/build/headers" -output "$apple/build/MeshwayCore.xcframework"
