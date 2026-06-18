#!/usr/bin/env bash
# Assemble a per-OS distributable that carries the application icon.
#
#   scripts/package.sh macos   -> target/package/Shadowcat.app  (icon.icns)
#   scripts/package.sh linux   -> target/package/shadowcat/     (.desktop + hicolor PNG)
#
# Windows needs no packaging here: the icon is embedded in the .exe at build
# time (see src/server/build.rs). Run after `cargo build --release`.
set -euo pipefail

os="${1:?usage: package.sh <macos|linux>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release/shadowcat"
out="$root/target/package"

[ -x "$bin" ] || { echo "missing release binary: $bin (run cargo build --release)" >&2; exit 1; }
rm -rf "$out"
mkdir -p "$out"

case "$os" in
  macos)
    app="$out/Shadowcat.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$root/packaging/macos/Info.plist" "$app/Contents/Info.plist"
    cp "$root/assets/icon.icns" "$app/Contents/Resources/icon.icns"
    cp "$bin" "$app/Contents/MacOS/shadowcat"
    chmod +x "$app/Contents/MacOS/shadowcat"
    echo "built $app"
    ;;
  linux)
    stage="$out/shadowcat"
    mkdir -p "$stage/bin" \
             "$stage/share/applications" \
             "$stage/share/icons/hicolor/256x256/apps"
    cp "$bin" "$stage/bin/shadowcat"
    chmod +x "$stage/bin/shadowcat"
    cp "$root/packaging/linux/shadowcat.desktop" "$stage/share/applications/shadowcat.desktop"
    cp "$root/assets/icon-256.png" "$stage/share/icons/hicolor/256x256/apps/shadowcat.png"
    echo "built $stage"
    ;;
  *)
    echo "unknown os: $os (expected macos or linux)" >&2
    exit 1
    ;;
esac
