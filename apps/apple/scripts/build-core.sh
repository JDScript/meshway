#!/bin/bash
set -euo pipefail
APPLE="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$APPLE/../.." && pwd)"
export CARGO_TARGET_DIR="$APPLE/build/rust"
cd "$ROOT"
for target in aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim; do
  cargo build --locked -p meshway-apple-core --release --target "$target"
done
mkdir -p "$APPLE/build/bindings" "$APPLE/MeshwayKit/Sources/MeshwayBindings"
cargo run --locked -p meshway-apple-core --features bindgen --bin meshway-apple-bindgen -- \
  generate --library "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/libmeshway_apple_core.dylib" \
  --language swift --out-dir "$APPLE/build/bindings"
cp "$APPLE/build/bindings/meshway_apple_core.swift" "$APPLE/MeshwayKit/Sources/MeshwayBindings/"
mkdir -p "$APPLE/build/headers"
cp "$APPLE/build/bindings/meshway_apple_coreFFI.h" "$APPLE/build/headers/"
cp "$APPLE/build/bindings/meshway_apple_coreFFI.modulemap" "$APPLE/build/headers/module.modulemap"
rm -rf "$APPLE/build/MeshwayCore.xcframework"
xcodebuild -create-xcframework \
  -library "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/libmeshway_apple_core.a" -headers "$APPLE/build/headers" \
  -library "$CARGO_TARGET_DIR/aarch64-apple-ios/release/libmeshway_apple_core.a" -headers "$APPLE/build/headers" \
  -library "$CARGO_TARGET_DIR/aarch64-apple-ios-sim/release/libmeshway_apple_core.a" -headers "$APPLE/build/headers" \
  -output "$APPLE/build/MeshwayCore.xcframework"
