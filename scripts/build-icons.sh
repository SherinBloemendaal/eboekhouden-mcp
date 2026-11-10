#!/usr/bin/env bash
# Renders every app icon file from src-tauri/icons/app.svg and app-small.svg.
# Needs rsvg-convert (brew install librsvg), iconutil (macOS) and the Tauri CLI from node_modules.
# Tauri draws the Windows and Store files from one source; the macOS icon.icns and the 16 and 32 px
# PNGs are built here so those sizes get the simplified drawing instead of a blurry scaled-down one.
set -euo pipefail
cd "$(dirname "$0")/../src-tauri/icons"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

render() { rsvg-convert -w "$2" -h "$2" "$1" -o "$3"; }

render app.svg 1024 app-1024.png
npx --no-install tauri icon app-1024.png -o "$tmp/tauri" >"$tmp/tauri.log" 2>&1 || { cat "$tmp/tauri.log"; exit 1; }
for f in icon.ico icon.png 64x64.png 128x128.png 128x128@2x.png StoreLogo.png Square*Logo.png; do
  cp "$tmp/tauri/$f" .
done

# Small sizes: the simplified drawing.
render app-small.svg 32 32x32.png
render app-small.svg 30 Square30x30Logo.png
render app-small.svg 44 Square44x44Logo.png

set="$tmp/icon.iconset"
mkdir "$set"
render app-small.svg 16 "$set/icon_16x16.png"
render app-small.svg 32 "$set/icon_16x16@2x.png"
render app-small.svg 32 "$set/icon_32x32.png"
render app.svg 64 "$set/icon_32x32@2x.png"
for px in 128 256 512; do
  render app.svg "$px" "$set/icon_${px}x${px}.png"
  render app.svg "$((px * 2))" "$set/icon_${px}x${px}@2x.png"
done
iconutil -c icns -o icon.icns "$set"

# The in-app logo (shown at up to 56 pt).
render app.svg 128 ../../public/app-icon.png
