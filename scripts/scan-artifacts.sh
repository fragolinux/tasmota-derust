#!/usr/bin/env bash
# Scans the release archives with Trivy, the way they will be published.
#   scripts/scan-artifacts.sh <release dir> <report dir> [android gradle.lockfile]
#
# Each archive is unpacked and scanned as a root filesystem: Rust binaries built with
# cargo-auditable carry their dependency list, so Trivy sees exactly what was shipped. For
# the APK the native library is scanned that way, and the Java/Kotlin libraries through the
# Gradle lockfile of the release build.
#
# Writes, per artifact: <name>.trivy.json (full result), <name>.trivy.txt (table) and
# <name>.cdx.json (CycloneDX SBOM), plus SUMMARY.md. Exits with 1 when a fixable HIGH or
# CRITICAL vulnerability is found, or a binary has no dependency data (SCAN_NO_FAIL=1 to
# only report).
set -euo pipefail

rel=$1 out=$2
root=$(cd "$(dirname "$0")/.." && pwd)
lock=${3:-$root/android/app/gradle.lockfile}

command -v trivy >/dev/null || { echo "trivy not found (brew install trivy)" >&2; exit 1; }
mkdir -p "$out"
rm -f "$out"/*.trivy.json "$out"/*.trivy.txt "$out"/*.cdx.json "$out"/SUMMARY.md
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

scan() { # scan <mode> <label> <dir>
    trivy "$1" --quiet --scanners vuln --list-all-pkgs --format json --output "$out/$2.trivy.json" "$3"
}

shopt -s nullglob
for f in "$rel"/*.zip "$rel"/*.tar.gz "$rel"/*.apk; do
    name=$(basename "$f")
    d="$stage/$name"
    mkdir -p "$d"
    case "$name" in
        *.zip) unzip -q "$f" -d "$d" ;;
        *.tar.gz) tar -xzf "$f" -C "$d" ;;
        # Trivy only looks at executable files; libraries in an APK have no mode bits.
        *.apk) unzip -q "$f" 'lib/*' -d "$d" && find "$d" -name '*.so' -exec chmod +x {} + ;;
    esac
    echo "Scanning $name"
    scan rootfs "$name" "$d"
    if [[ "$name" == *.apk ]]; then
        if [ -f "$lock" ]; then
            mkdir -p "$stage/java"
            cp "$lock" "$stage/java/gradle.lockfile"
            scan fs "$name.java-libraries" "$stage/java"
        else
            echo "warning: $lock not found, Java/Kotlin libraries of the APK not scanned" >&2
        fi
    fi
done

reports=("$out"/*.trivy.json)
[ ${#reports[@]} -gt 0 ] || { echo "Nothing to scan in $rel (run make package first)." >&2; exit 1; }
for j in "${reports[@]}"; do
    base=${j%.trivy.json}
    trivy convert --quiet --format table --output "$base.trivy.txt" "$j"
    trivy convert --quiet --format cyclonedx --output "$base.cdx.json" "$j"
done

trivy_info=$(trivy version --format json)
python3 "$root/scripts/trivy_summary.py" "$out" "$trivy_info" > "$out/SUMMARY.md" && status=0 || status=$?
cat "$out/SUMMARY.md"
if [ "$status" -ne 0 ] && [ "${SCAN_NO_FAIL:-0}" != 1 ]; then
    echo "Security scan failed: see $out/SUMMARY.md" >&2
    exit 1
fi
