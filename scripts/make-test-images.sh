#!/bin/bash
# Build FAT32 / exFAT / HFS+ / APFS disk images on macOS, write sample files into
# "Photos/2024", delete them, and leave the images for Restorix to scan.
#   scripts/make-test-images.sh <output-dir>
set -euo pipefail
OUT="${1:-test-images}"
HERE="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$OUT"
SAMPLES="$OUT/samples"
python3 -I "$HERE/make_samples.py" "$SAMPLES"

make_image() {
  local fs="$1" name="$2"
  local img="$OUT/$name.dmg"
  rm -f "$img"
  hdiutil create -quiet -size 64m -fs "$fs" -volname "T$name" -layout GPTSPUD "$img"
  local mnt
  mnt=$(hdiutil attach -nobrowse -noautoopen "$img" | awk -F'\t' '/\/Volumes\//{print $NF}' | tail -1)
  mkdir -p "$mnt/Photos/2024" "$mnt/Keep"
  cp "$SAMPLES"/* "$mnt/Photos/2024/"
  cp "$SAMPLES/notes.txt" "$mnt/Keep/still-here.txt"
  sync; sleep 1
  rm -rf "$mnt/Photos"
  sync; sleep 1
  hdiutil detach -quiet "$mnt"
  echo "built $img"
}

make_image "MS-DOS FAT32" fat32
make_image "ExFAT" exfat
make_image "HFS+" hfsplus
make_image "APFS" apfs
