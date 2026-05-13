#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <dmg-path> <icon-icns-path>" >&2
  exit 1
fi

DMG_PATH="$1"
ICON_PATH="$2"
TMP_RSRC="$(mktemp -t telegram-drive-dmg-icon.XXXXXX)"
cleanup() {
  rm -f "$TMP_RSRC"
}
trap cleanup EXIT

sips -i "$ICON_PATH" >/dev/null
DeRez -only icns "$ICON_PATH" > "$TMP_RSRC"
Rez -append "$TMP_RSRC" -o "$DMG_PATH"
SetFile -a C "$DMG_PATH"

echo "Branded DMG icon: $DMG_PATH"
