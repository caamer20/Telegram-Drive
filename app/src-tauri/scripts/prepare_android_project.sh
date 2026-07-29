#!/usr/bin/env sh
set -eu

generated_dir="${1:-gen/android}"
gradle_file="$generated_dir/app/build.gradle.kts"
manifest_file="$generated_dir/app/src/main/AndroidManifest.xml"
ndk_version="${ANDROID_NDK_VERSION:-27.2.12479018}"

if [ ! -f "$gradle_file" ]; then
  echo "generated Android Gradle file not found: $gradle_file" >&2
  exit 1
fi
if [ ! -f "$manifest_file" ]; then
  echo "generated Android manifest not found: $manifest_file" >&2
  exit 1
fi

# Tauri's generated app defaults to global cleartext and advertises a TV launcher
# without the required TV assets. Native streaming only needs the plugin's
# destination-scoped 127.0.0.1 network security exception, and this app does not
# ship a TV-specific UI.
sed -i.bak \
  -e 's/manifestPlaceholders\["usesCleartextTraffic"\] = "true"/manifestPlaceholders["usesCleartextTraffic"] = "false"/' \
  "$gradle_file"
rm -f "$gradle_file.bak"

if ! grep -Fq 'ndkVersion = ' "$gradle_file"; then
  sed -i.bak \
    -e "/^android {/a\\
    ndkVersion = \"$ndk_version\"
" \
    "$gradle_file"
  rm -f "$gradle_file.bak"
fi

sed -i.bak \
  -e '/AndroidTV support/d' \
  -e '/android\.software\.leanback/d' \
  -e '/android\.intent\.category\.LEANBACK_LAUNCHER/d' \
  "$manifest_file"
rm -f "$manifest_file.bak"

if ! grep -Fq 'manifestPlaceholders["usesCleartextTraffic"] = "false"' "$gradle_file"; then
  echo "Tauri cleartext manifest placeholder was not found" >&2
  exit 1
fi
if ! grep -Fq "ndkVersion = \"$ndk_version\"" "$gradle_file"; then
  echo "Pinned Android NDK version was not applied" >&2
  exit 1
fi
if grep -Fq 'LEANBACK_LAUNCHER' "$manifest_file"; then
  echo "generated Android manifest still advertises an unsupported TV launcher" >&2
  exit 1
fi

echo "Prepared generated Android project: $generated_dir"
