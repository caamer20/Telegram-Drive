#!/usr/bin/env bash
set -euo pipefail
bundle="$(cd "${1:?Usage: smoke-packaged-artifacts.sh BUNDLE_DIRECTORY}" && pwd)"
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
mounted=""
trap 'if [[ -n "$mounted" ]]; then hdiutil detach "$mounted" || true; fi; rm -rf "$work"' EXIT
smoke() {
  # Exercise the numeric-leading regression using a fresh nonce for every app.
  local token
  token="$(node -e 'process.stdout.write("0" + require("node:crypto").randomBytes(16).toString("hex").slice(1))')"
  if [[ "$(uname -s)" == Darwin ]]; then
    node "$repository/scripts/packaged-startup-smoke.cjs" --disposable-user --executable "$1" --run-token "$token" --expected-bundle-type "$2"
  else
    xvfb-run -a dbus-run-session -- node "$repository/scripts/packaged-startup-smoke.cjs" --disposable-user --executable "$1" --run-token "$token" --expected-bundle-type "$2"
  fi
}
one_app() {
  local applications=()
  while IFS= read -r file; do applications+=("$file"); done < <(find "$1" -type f -path '*/Contents/MacOS/app' -print)
  [[ ${#applications[@]} -eq 1 ]] || { echo 'Expected exactly one packaged macOS executable' >&2; exit 1; }
  smoke "${applications[0]}" app
}
if [[ "$(uname -s)" == Darwin ]]; then
  dmgs=0
  while IFS= read -r artifact; do
    dmgs=$((dmgs + 1)); mounted="$work/mount-$dmgs"; mkdir "$mounted"
    hdiutil attach -readonly -nobrowse -mountpoint "$mounted" "$artifact"
    one_app "$mounted"
    hdiutil detach "$mounted"; mounted=""
  done < <(find "$bundle" -type f -name '*.dmg' -print)
  archives=0
  while IFS= read -r artifact; do
    archives=$((archives + 1)); directory="$work/archive-$archives"; mkdir "$directory"
    tar -xzf "$artifact" -C "$directory"; one_app "$directory"
  done < <(find "$bundle" -type f -name '*.app.tar.gz' -print)
  [[ $dmgs -gt 0 && $archives -gt 0 ]] || { echo 'Missing DMG or updater application archive' >&2; exit 1; }
else
  for kind in AppImage deb rpm; do
    count=0
    while IFS= read -r artifact; do
      count=$((count + 1)); directory="$work/$kind-$count"; mkdir "$directory"
      case "$kind" in
        AppImage) chmod +x "$artifact"; (cd "$directory" && "$artifact" --appimage-extract >/dev/null); executable="$directory/squashfs-root/AppRun" ;;
        deb) dpkg-deb -x "$artifact" "$directory"; executable="$directory/usr/bin/app" ;;
        rpm) bsdtar -xf "$artifact" -C "$directory"; executable="$directory/usr/bin/app" ;;
      esac
      [[ -f "$executable" ]] || { echo "Missing packaged executable in $artifact" >&2; exit 1; }
      smoke "$executable" "$(echo "$kind" | tr '[:upper:]' '[:lower:]')"
    done < <(find "$bundle" -type f -name "*.$kind" -print)
    [[ $count -gt 0 ]] || { echo "Missing $kind application artifact" >&2; exit 1; }
  done
fi
