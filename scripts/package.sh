#!/usr/bin/env bash
# Packs whatever `make` built in dist/ into release files, scanning each one with Trivy.
#   scripts/package.sh <dist dir> <release dir> <report dir> <version>
#
# Release files:
#   macOS    desktop app: .dmg (drag the app to Applications), web server: .zip
#   Windows  .zip
#   Linux    .tar.gz
#   Android  the .apk alone
#
# Every archive and disk image contains, next to the program: LICENSE-MIT, LICENSE-APACHE,
# TRIVY-REPORT.txt / TRIVY-REPORT.json (Trivy scan of exactly those files) and SHA256SUMS
# (checksums of every file inside, check with `shasum -a 256 -c SHA256SUMS`). The executable
# bit is kept. Rust binaries built with cargo-auditable carry their dependency list, so Trivy
# sees exactly what is shipped; for the APK the native library is scanned that way and the
# Java/Kotlin libraries through the Gradle lockfile of the release build.
#
# The report dir gets, per release file: <name>.trivy.json, <name>.trivy.txt, <name>.cdx.json
# (CycloneDX SBOM), plus trivy-version.json and SUMMARY.md. Exits with 1, packing nothing,
# when a fixable HIGH or CRITICAL vulnerability is found or a binary has no dependency data
# (SCAN_NO_FAIL=1 to only report).
set -euo pipefail

dist=$1 out=$2 reports=$3 version=$4
root=$(cd "$(dirname "$0")/.." && pwd)
lock=${ANDROID_LOCKFILE:-$root/android/app/gradle.lockfile}

command -v trivy >/dev/null || { echo "trivy not found (brew install trivy)" >&2; exit 1; }
rm -rf "$out" "$reports"
mkdir -p "$out" "$reports"
out=$(cd "$out" && pwd)
reports=$(cd "$reports" && pwd)
trivy version --format json > "$reports/trivy-version.json"
dist=$(cd "$dist" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

trivy_scan() { # trivy_scan <mode> <label> <path>
    trivy "$1" --quiet --scanners vuln --list-all-pkgs --format json \
        --output "$reports/$2.trivy.json" "$3"
    trivy convert --quiet --format table --output "$reports/$2.trivy.txt" "$reports/$2.trivy.json"
    trivy convert --quiet --format cyclonedx --output "$reports/$2.cdx.json" "$reports/$2.trivy.json"
}

# prepare <release file name> <files...>: stage dir with the files, licenses, Trivy report
# and SHA256SUMS. Prints the stage dir.
prepare() {
    local name=$1; shift
    local d="$stage/$name"
    mkdir -p "$d"
    cp -R "$@" "$d/"
    cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$d/"
    echo "Scanning $name" >&2
    trivy_scan rootfs "$name" "$d" >&2
    python3 "$root/scripts/trivy_summary.py" --report "$reports/$name.trivy.json" \
        "$reports/trivy-version.json" > "$d/TRIVY-REPORT.txt"
    cp "$reports/$name.trivy.json" "$d/TRIVY-REPORT.json"
    (cd "$d" && find . -type f ! -name SHA256SUMS | LC_ALL=C sort | sed 's|^\./||' \
        | while IFS= read -r f; do sha256 "$f"; done > SHA256SUMS)
    echo "$d"
}

# Every release file is staged and scanned first; nothing is packed if the scan blocks.
jobs=()
add() { # add <kind> <release file name> <files...>
    local kind=$1 name=$2; shift 2
    jobs+=("$kind|$name|$(prepare "$name" "$@")")
}

app="$dist/mac/Tasmota Derust.app"
[ -d "$app" ] && add dmg "tasmota-derust-$version-macos-arm64.dmg" "$app"
[ -f "$dist/mac/tasmota-derust-web" ] &&
    add zip "tasmota-derust-web-$version-macos-arm64.zip" "$dist/mac/tasmota-derust-web"
[ -f "$dist/windows/tasmota-derust.exe" ] &&
    add zip "tasmota-derust-$version-windows-x64.zip" "$dist/windows/tasmota-derust.exe"
[ -f "$dist/windows/tasmota-derust-web.exe" ] &&
    add zip "tasmota-derust-web-$version-windows-x64.zip" "$dist/windows/tasmota-derust-web.exe"
[ -f "$dist/linux/tasmota-derust" ] &&
    add tgz "tasmota-derust-$version-linux-x64.tar.gz" "$dist/linux/tasmota-derust"
[ -f "$dist/linux/tasmota-derust-web" ] &&
    add tgz "tasmota-derust-web-$version-linux-x64.tar.gz" "$dist/linux/tasmota-derust-web"

apk="$dist/android/tasmota-derust.apk"
apk_name="tasmota-derust-$version.apk"
if [ -f "$apk" ]; then
    d="$stage/$apk_name"
    mkdir -p "$d"
    # Trivy only looks at executable files; libraries in an APK have no mode bits.
    unzip -q "$apk" 'lib/*' -d "$d" && find "$d" -name '*.so' -exec chmod +x {} +
    echo "Scanning $apk_name"
    trivy_scan rootfs "$apk_name" "$d"
    if [ -f "$lock" ]; then
        mkdir -p "$stage/java"
        cp "$lock" "$stage/java/gradle.lockfile"
        trivy_scan fs "$apk_name.java-libraries" "$stage/java"
    else
        echo "warning: $lock not found, Java/Kotlin libraries of the APK not scanned" >&2
    fi
fi

if [ ${#jobs[@]} -eq 0 ] && [ ! -f "$apk" ]; then
    echo "Nothing to package: build something first (make mac, make windows, ...)." >&2
    exit 1
fi

python3 "$root/scripts/trivy_summary.py" "$reports" > "$reports/SUMMARY.md" && status=0 || status=$?
cat "$reports/SUMMARY.md"
if [ "$status" -ne 0 ] && [ "${SCAN_NO_FAIL:-0}" != 1 ]; then
    echo "Security scan failed, nothing packed: see $reports/SUMMARY.md" >&2
    exit 1
fi

for job in ${jobs[@]+"${jobs[@]}"}; do
    IFS='|' read -r kind name d <<< "$job"
    case "$kind" in
        zip) (cd "$d" && zip -qry "$out/$name" .) ;;
        tgz) tar -C "$d" -czf "$out/$name" . ;;
        dmg)
            ln -s /Applications "$d/Applications"
            hdiutil create -quiet -volname "Tasmota Derust $version" -srcfolder "$d" \
                -fs HFS+ -format UDZO -ov "$out/$name" ;;
    esac
done
[ -f "$apk" ] && cp "$apk" "$out/$apk_name"

cd "$out"
files=(*)
sha256 "${files[@]}" > SHA256SUMS
ls -lh "${files[@]}" SHA256SUMS | awk '{print $5 "\t" $9}'
