#!/bin/bash
# Package the built Restorix.app into a compressed DMG without driving Finder
# (tauri's styled-DMG step needs Finder automation, which fails on headless/CI-less shells).
set -euo pipefail
cd "$(dirname "$0")/.."
APP=$(ls -d target/release/bundle/macos/Restorix.app target/*/release/bundle/macos/Restorix.app 2>/dev/null | head -1 || true)
[ -n "$APP" ] || { echo "Build the app first: npx tauri build --bundles app"; exit 1; }
VER=$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")
ARCH=$(uname -m)
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
OUT="target/release/bundle/dmg/Restorix_${VER}_${ARCH}.dmg"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
hdiutil create -quiet -volname "Restorix" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
rm -rf "$STAGE"
echo "$OUT"
