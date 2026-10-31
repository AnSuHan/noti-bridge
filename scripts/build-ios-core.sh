#!/usr/bin/env bash
# macOS 전용: Rust 코어를 XCFramework 로 묶고 Swift 바인딩을 생성한다.
set -euo pipefail
cd "$(dirname "$0")/.."

rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo build --release -p noti-bridge-ffi --lib --target aarch64-apple-ios
cargo build --release -p noti-bridge-ffi --lib --target aarch64-apple-ios-sim
cargo build --release -p noti-bridge-ffi --lib --target x86_64-apple-ios

GEN=ios/Generated
rm -rf "$GEN" ios/NotiBridgeCore.xcframework
mkdir -p "$GEN/headers" target/ios-sim-universal
cargo run -q -p noti-bridge-ffi --bin uniffi-bindgen -- generate \
  --library target/aarch64-apple-ios/release/libnoti_bridge_ffi.a \
  --language swift --no-format --out-dir "$GEN"
mv "$GEN"/*.h "$GEN/headers/"
cat "$GEN"/*.modulemap > "$GEN/headers/module.modulemap"
rm "$GEN"/*.modulemap

lipo -create \
  target/aarch64-apple-ios-sim/release/libnoti_bridge_ffi.a \
  target/x86_64-apple-ios/release/libnoti_bridge_ffi.a \
  -output target/ios-sim-universal/libnoti_bridge_ffi.a
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libnoti_bridge_ffi.a -headers "$GEN/headers" \
  -library target/ios-sim-universal/libnoti_bridge_ffi.a -headers "$GEN/headers" \
  -output ios/NotiBridgeCore.xcframework
