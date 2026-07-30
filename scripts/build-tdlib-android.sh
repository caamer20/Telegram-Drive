#!/usr/bin/env bash
set -euo pipefail

# Reproducible Phase 0 TDLib build. Only official Telegram/OpenSSL sources are used.
TDLIB_COMMIT="022d60202e446ad1287b9fb68e687c8a0760788b"
OPENSSL_TAG="OpenSSL_1_1_1w"
NDK_VERSION="27.2.12479018"
ABIS=(arm64-v8a x86_64)

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
BUILD_ROOT="${TDLIB_BUILD_ROOT:-$REPO_ROOT/android-app/tdlib-build}"
OUTPUT_ROOT="$REPO_ROOT/android-app/app/src/main/jniLibs"

if [[ -z "$SDK_ROOT" || ! -d "$SDK_ROOT/ndk/$NDK_VERSION" ]]; then
  echo "Android NDK $NDK_VERSION is required" >&2
  exit 1
fi

mkdir -p "$BUILD_ROOT" "$OUTPUT_ROOT"
if [[ ! -d "$BUILD_ROOT/td/.git" ]]; then
  git clone https://github.com/tdlib/td.git "$BUILD_ROOT/td"
fi
git -C "$BUILD_ROOT/td" fetch --depth 1 origin "$TDLIB_COMMIT"
git -C "$BUILD_ROOT/td" checkout --detach "$TDLIB_COMMIT"

if [[ ! -d "$BUILD_ROOT/openssl-src" ]]; then
  curl -fsSL "https://github.com/openssl/openssl/archive/refs/tags/$OPENSSL_TAG.tar.gz" -o "$BUILD_ROOT/openssl.tar.gz"
  mkdir "$BUILD_ROOT/openssl-src"
  tar xzf "$BUILD_ROOT/openssl.tar.gz" --strip-components=1 -C "$BUILD_ROOT/openssl-src"
fi

TOOLCHAIN="$SDK_ROOT/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/darwin-x86_64"
export ANDROID_NDK_HOME="$SDK_ROOT/ndk/$NDK_VERSION"
export PATH="$TOOLCHAIN/bin:$SDK_ROOT/cmake/3.22.1/bin:$PATH"

# TDLib's official Android flow generates TL sources with a host build first.
HOST_BUILD="$BUILD_ROOT/td-build-host"
cmake -S "$BUILD_ROOT/td/example/android" -B "$HOST_BUILD" -DTD_GENERATE_SOURCE_FILES=ON
cmake --build "$HOST_BUILD" -j4

for ABI in "${ABIS[@]}"; do
  OPENSSL_PREFIX="$BUILD_ROOT/openssl/$ABI"
  if [[ ! -f "$OPENSSL_PREFIX/lib/libcrypto.a" ]]; then
    pushd "$BUILD_ROOT/openssl-src" >/dev/null
    make distclean >/dev/null 2>&1 || true
    if [[ "$ABI" == "arm64-v8a" ]]; then
      TARGET="android-arm64"
    else
      TARGET="android-x86_64"
    fi
    LDFLAGS=-Wl,-z,max-page-size=16384 ./Configure "$TARGET" no-shared -D__ANDROID_API__=26 --prefix="$OPENSSL_PREFIX"
    make -j4
    make install_sw
    popd >/dev/null
  fi

  BUILD_DIR="$BUILD_ROOT/td-build-$ABI"
  cmake -S "$BUILD_ROOT/td/example/android" -B "$BUILD_DIR" -GNinja \
    -DCMAKE_TOOLCHAIN_FILE="$SDK_ROOT/ndk/$NDK_VERSION/build/cmake/android.toolchain.cmake" \
    -DOPENSSL_ROOT_DIR="$OPENSSL_PREFIX" \
    -DCMAKE_BUILD_TYPE=RelWithDebInfo \
    -DANDROID_ABI="$ABI" \
    -DANDROID_STL=c++_static \
    -DANDROID_PLATFORM=android-26 \
    -DTD_ANDROID_JSON_JAVA=ON
  cmake --build "$BUILD_DIR" --target tdjni -j4
  mkdir -p "$OUTPUT_ROOT/$ABI"
  cp "$BUILD_DIR/libtdjsonjava.so" "$OUTPUT_ROOT/$ABI/libtdjsonjava.so"
done

echo "TDLib $TDLIB_COMMIT built for: ${ABIS[*]}"
