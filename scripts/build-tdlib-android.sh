#!/usr/bin/env bash
set -euo pipefail

# Source-only TDLib Android build. Pins are immutable and verified before build.
TDLIB_COMMIT="022d60202e446ad1287b9fb68e687c8a0760788b"
OPENSSL_VERSION="3.5.7"
OPENSSL_SHA256="a8c0d28a529ca480f9f36cf5792e2cd21984552a3c8e4aa11a24aa31aeac98e8"
NDK_VERSION="27.2.12479018"
CMAKE_VERSION="3.22.1"
ANDROID_API="26"
ABIS=(arm64-v8a x86_64)

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
BUILD_ROOT="${TDLIB_BUILD_ROOT:-$REPO_ROOT/android-app/tdlib-build}"
OUTPUT_ROOT="$REPO_ROOT/android-app/app/src/main/jniLibs"
METADATA_FILE="$REPO_ROOT/android-app/tdlib-build-metadata.txt"
OPENSSL_ARCHIVE="$BUILD_ROOT/openssl-$OPENSSL_VERSION.tar.gz"
OPENSSL_SOURCE="$BUILD_ROOT/openssl-$OPENSSL_VERSION-src"

fail() { printf 'TDLib Android build: %s\n' "$*" >&2; exit 1; }
require_command() { command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"; }

[[ -n "$SDK_ROOT" ]] || fail "set ANDROID_SDK_ROOT or ANDROID_HOME"
[[ -d "$SDK_ROOT/ndk/$NDK_VERSION" ]] || fail "Android NDK $NDK_VERSION is required at $SDK_ROOT/ndk/$NDK_VERSION"
for command_name in git curl tar perl make shasum; do require_command "$command_name"; done

CMAKE_BIN="$SDK_ROOT/cmake/$CMAKE_VERSION/bin/cmake"
NINJA_BIN="$SDK_ROOT/cmake/$CMAKE_VERSION/bin/ninja"
[[ -x "$CMAKE_BIN" ]] || fail "Android SDK CMake $CMAKE_VERSION is required"
[[ -x "$NINJA_BIN" ]] || fail "Android SDK Ninja bundled with CMake $CMAKE_VERSION is required"

case "$(uname -s)" in
  Darwin) host_prefix="darwin" ;;
  Linux) host_prefix="linux" ;;
  *) fail "unsupported build host: $(uname -s)" ;;
esac
TOOLCHAIN_ROOT="$SDK_ROOT/ndk/$NDK_VERSION/toolchains/llvm/prebuilt"
TOOLCHAIN=""
for candidate in "$TOOLCHAIN_ROOT"/"$host_prefix"-*; do
  [[ -d "$candidate/bin" ]] || continue
  [[ -z "$TOOLCHAIN" ]] || fail "multiple NDK host toolchains found under $TOOLCHAIN_ROOT"
  TOOLCHAIN="$candidate"
done
[[ -n "$TOOLCHAIN" ]] || fail "no NDK host toolchain found for $host_prefix under $TOOLCHAIN_ROOT"

mkdir -p "$BUILD_ROOT" "$OUTPUT_ROOT"
if [[ ! -d "$BUILD_ROOT/td/.git" ]]; then
  git clone https://github.com/tdlib/td.git "$BUILD_ROOT/td"
fi
git -C "$BUILD_ROOT/td" fetch --depth 1 origin "$TDLIB_COMMIT"
git -C "$BUILD_ROOT/td" checkout --detach "$TDLIB_COMMIT"
[[ "$(git -C "$BUILD_ROOT/td" rev-parse HEAD)" == "$TDLIB_COMMIT" ]] || fail "TDLib checkout integrity verification failed"

if [[ ! -f "$OPENSSL_ARCHIVE" ]]; then
  curl -fsSL "https://github.com/openssl/openssl/releases/download/openssl-$OPENSSL_VERSION/openssl-$OPENSSL_VERSION.tar.gz" -o "$OPENSSL_ARCHIVE"
fi
actual_openssl_sha="$(shasum -a 256 "$OPENSSL_ARCHIVE" | awk '{print $1}')"
[[ "$actual_openssl_sha" == "$OPENSSL_SHA256" ]] || fail "OpenSSL archive checksum mismatch: $actual_openssl_sha"
if [[ ! -f "$OPENSSL_SOURCE/.source-sha256" ]] || [[ "$(<"$OPENSSL_SOURCE/.source-sha256")" != "$OPENSSL_SHA256" ]]; then
  rm -rf "$OPENSSL_SOURCE"
  mkdir -p "$OPENSSL_SOURCE"
  tar xzf "$OPENSSL_ARCHIVE" --strip-components=1 -C "$OPENSSL_SOURCE"
  printf '%s\n' "$OPENSSL_SHA256" > "$OPENSSL_SOURCE/.source-sha256"
fi

export ANDROID_NDK_HOME="$SDK_ROOT/ndk/$NDK_VERSION"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
export PATH="$TOOLCHAIN/bin:$SDK_ROOT/cmake/$CMAKE_VERSION/bin:$PATH"

# Generate TL sources once with the host compiler, as required by TDLib's Android build.
HOST_BUILD="$BUILD_ROOT/td-build-host-$TDLIB_COMMIT"
"$CMAKE_BIN" -S "$BUILD_ROOT/td/example/android" -B "$HOST_BUILD" -DTD_GENERATE_SOURCE_FILES=ON
"$CMAKE_BIN" --build "$HOST_BUILD" --parallel 4

for ABI in "${ABIS[@]}"; do
  OPENSSL_PREFIX="$BUILD_ROOT/openssl-$OPENSSL_VERSION/$ABI"
  if [[ ! -f "$OPENSSL_PREFIX/lib/libcrypto.a" ]]; then
    pushd "$OPENSSL_SOURCE" >/dev/null
    make distclean >/dev/null 2>&1 || true
    case "$ABI" in
      arm64-v8a) openssl_target="android-arm64" ;;
      x86_64) openssl_target="android-x86_64" ;;
      *) fail "unsupported ABI: $ABI" ;;
    esac
    LDFLAGS="-Wl,-z,max-page-size=16384" ./Configure "$openssl_target" no-shared no-tests \
      -D__ANDROID_API__="$ANDROID_API" --prefix="$OPENSSL_PREFIX"
    make -j4
    make install_sw
    popd >/dev/null
  fi

  BUILD_DIR="$BUILD_ROOT/td-build-$ABI-openssl-$OPENSSL_VERSION"
  "$CMAKE_BIN" -S "$BUILD_ROOT/td/example/android" -B "$BUILD_DIR" -GNinja \
    -DCMAKE_MAKE_PROGRAM="$NINJA_BIN" \
    -DCMAKE_TOOLCHAIN_FILE="$SDK_ROOT/ndk/$NDK_VERSION/build/cmake/android.toolchain.cmake" \
    -DOPENSSL_ROOT_DIR="$OPENSSL_PREFIX" \
    -DCMAKE_BUILD_TYPE=RelWithDebInfo \
    -DANDROID_ABI="$ABI" \
    -DANDROID_STL=c++_static \
    -DANDROID_PLATFORM="android-$ANDROID_API" \
    -DTD_ANDROID_JSON_JAVA=ON
  "$CMAKE_BIN" --build "$BUILD_DIR" --target tdjni --parallel 4
  install -d "$OUTPUT_ROOT/$ABI"
  install -m 0644 "$BUILD_DIR/libtdjsonjava.so" "$OUTPUT_ROOT/$ABI/libtdjsonjava.so"
done

{
  printf 'format=telegram-drive-tdlib-build-v1\n'
  printf 'tdlib_repository=https://github.com/tdlib/td.git\n'
  printf 'tdlib_commit=%s\n' "$TDLIB_COMMIT"
  printf 'openssl_source=https://github.com/openssl/openssl/releases/download/openssl-%s/openssl-%s.tar.gz\n' "$OPENSSL_VERSION" "$OPENSSL_VERSION"
  printf 'openssl_version=%s\n' "$OPENSSL_VERSION"
  printf 'openssl_source_sha256=%s\n' "$OPENSSL_SHA256"
  printf 'ndk_version=%s\n' "$NDK_VERSION"
  printf 'cmake_version=%s\n' "$CMAKE_VERSION"
  printf 'android_api=%s\n' "$ANDROID_API"
  printf 'build_host=%s-%s\n' "$(uname -s)" "$(uname -m)"
  for ABI in "${ABIS[@]}"; do
    printf 'binary.%s.path=app/src/main/jniLibs/%s/libtdjsonjava.so\n' "$ABI" "$ABI"
    printf 'binary.%s.sha256=%s\n' "$ABI" "$(shasum -a 256 "$OUTPUT_ROOT/$ABI/libtdjsonjava.so" | awk '{print $1}')"
  done
} > "$METADATA_FILE"

printf 'TDLib %s with OpenSSL %s built for: %s\nMetadata: %s\n' \
  "$TDLIB_COMMIT" "$OPENSSL_VERSION" "${ABIS[*]}" "$METADATA_FILE"
