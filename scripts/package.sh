#!/usr/bin/env bash
# Packs whatever `make` built in dist/ into release archives, plus SHA256SUMS.
#   scripts/package.sh <dist dir> <output dir> <version>
set -euo pipefail

dist=$1 out=$2 version=$3
root=$(cd "$(dirname "$0")/.." && pwd)
rm -rf "$out"
mkdir -p "$out"
out=$(cd "$out" && pwd)
dist=$(cd "$dist" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

licenses() { cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$1/"; }

# zip_of <archive name> <files...>: zip with the licenses, files at the top level.
zip_of() {
    local name=$1; shift
    local d="$stage/$name"
    mkdir -p "$d"
    cp -R "$@" "$d/"
    licenses "$d"
    (cd "$d" && zip -qry "$out/$name.zip" .)
}

# tar_of <archive name> <files...>: same as a .tar.gz (keeps the executable bit).
tar_of() {
    local name=$1; shift
    local d="$stage/$name"
    mkdir -p "$d"
    cp -R "$@" "$d/"
    licenses "$d"
    tar -C "$d" -czf "$out/$name.tar.gz" .
}

if [ -d "$dist/mac/Tasmota Derust.app" ]; then
    zip_of "tasmota-derust-$version-macos-arm64" "$dist/mac/Tasmota Derust.app"
fi
if [ -f "$dist/mac/tasmota-derust-web" ]; then
    tar_of "tasmota-derust-web-$version-macos-arm64" "$dist/mac/tasmota-derust-web"
fi
if [ -f "$dist/windows/tasmota-derust.exe" ]; then
    zip_of "tasmota-derust-$version-windows-x64" "$dist/windows/tasmota-derust.exe"
fi
if [ -f "$dist/windows/tasmota-derust-web.exe" ]; then
    zip_of "tasmota-derust-web-$version-windows-x64" "$dist/windows/tasmota-derust-web.exe"
fi
if [ -f "$dist/linux/tasmota-derust" ]; then
    tar_of "tasmota-derust-$version-linux-x64" "$dist/linux/tasmota-derust"
fi
if [ -f "$dist/linux/tasmota-derust-web" ]; then
    tar_of "tasmota-derust-web-$version-linux-x64" "$dist/linux/tasmota-derust-web"
fi
if [ -f "$dist/android/tasmota-derust.apk" ]; then
    cp "$dist/android/tasmota-derust.apk" "$out/tasmota-derust-$version.apk"
fi

cd "$out"
shopt -s nullglob
files=(*.zip *.tar.gz *.apk)
if [ ${#files[@]} -eq 0 ]; then
    echo "Nothing to package: build something first (make mac, make windows, ...)." >&2
    exit 1
fi
if command -v sha256sum >/dev/null; then sha256sum "${files[@]}" > SHA256SUMS
else shasum -a 256 "${files[@]}" > SHA256SUMS; fi
ls -lh "${files[@]}" SHA256SUMS | awk '{print $5 "\t" $9}'
