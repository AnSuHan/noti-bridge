#!/usr/bin/env bash
# Rust 코어를 Android 용으로 빌드하고 Kotlin 바인딩을 생성한다.
set -euo pipefail
cd "$(dirname "$0")/.."

export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$(ls -d "${ANDROID_HOME:-$LOCALAPPDATA/Android/Sdk}"/ndk/* | sort -V | tail -1)}"
JNI_DIR=android/app/src/main/jniLibs
GEN_DIR=android/app/src/main/java

cargo ndk -t arm64-v8a -t x86_64 --platform 26 -o "$JNI_DIR" build --release -p noti-bridge-ffi --lib
# iroh 크레이트의 cdylib 부산물은 앱에 필요 없다.
rm -f "$JNI_DIR"/*/libiroh*.so

# 릴리스 빌드는 심볼을 제거해 UniFFI 메타데이터가 없으므로 호스트 디버그 빌드에서 바인딩을 만든다.
cargo build -p noti-bridge-ffi
for HOST_LIB in target/debug/noti_bridge_ffi.dll target/debug/libnoti_bridge_ffi.so target/debug/libnoti_bridge_ffi.dylib; do
  [ -f "$HOST_LIB" ] && break
done
cargo run -q -p noti-bridge-ffi --bin uniffi-bindgen -- generate   --library "$HOST_LIB" --language kotlin --no-format --out-dir "$GEN_DIR"
