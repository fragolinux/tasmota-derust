#!/usr/bin/env bash
# Regenerates the app icon files from packaging/icon/icon.svg (macOS only: headless Chrome
# renders the SVG, sips and iconutil make the sizes and the .icns). The results are committed,
# so normal builds don't need this. Android uses vector drawables with the same paths
# (android/app/src/main/res/drawable/ic_launcher_*.xml): update them by hand.
set -euo pipefail
cd "$(dirname "$0")/.."
chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

printf '<html><body style="margin:0">%s</body></html>' "$(cat packaging/icon/icon.svg)" > "$tmp/icon.html"
"$chrome" --headless --disable-gpu --hide-scrollbars --default-background-color=00000000 \
    --window-size=1024,1024 --screenshot="$tmp/icon.png" "file://$tmp/icon.html" 2>/dev/null
cp "$tmp/icon.png" packaging/icon/icon-1024.png

# Desktop window icon (embedded in the executable).
sips -z 256 256 "$tmp/icon.png" --out crates/desktop/assets/icon-256.png >/dev/null

# macOS .icns
set_dir="$tmp/AppIcon.iconset"
mkdir -p "$set_dir"
for s in 16 32 128 256 512; do
    sips -z $s $s "$tmp/icon.png" --out "$set_dir/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z $d $d "$tmp/icon.png" --out "$set_dir/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$set_dir" -o packaging/macos/AppIcon.icns

# Windows .ico: PNG-compressed entries (Vista and later).
for s in 16 24 32 48 64 128 256; do
    sips -z $s $s "$tmp/icon.png" --out "$tmp/ico_$s.png" >/dev/null
done
python3 -I - "$tmp" packaging/windows/icon.ico <<'PY'
import struct, sys
tmp, out = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
blobs = [open(f"{tmp}/ico_{s}.png", "rb").read() for s in sizes]
head = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
for s, b in zip(sizes, blobs):
    entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(b), offset)
    offset += len(b)
open(out, "wb").write(head + entries + b"".join(blobs))
PY
echo "Icons written."
