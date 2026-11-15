#!/usr/bin/env bash
# Makes a release from this Mac: checks, version bump, signed commit and tag, push.
#   scripts/release.sh [version]      (make release [VERSION=x.y.z] [DRY_RUN=1])
#
# Without a version, the next one comes from the Conventional Commits since the last tag
# pushed to origin:
#   feat           -> minor (0.1.3 -> 0.2.0)
#   fix, perf      -> patch (0.1.3 -> 0.1.4)
#   anything else  (docs, chore, ci, build, refactor, test, style, revert) -> no release
# A breaking change (`feat!:` or a `BREAKING CHANGE:` footer), a commit that does not follow
# Conventional Commits, or a pre-release as last tag stops it: give the version explicitly
# then. Majors are always explicit. An explicit version (also `0.2.0-rc1`) overrides the
# commits, e.g. to release a patch after a `feat` committed by mistake.
#
# Never waits for a person halfway: every secret is checked at the start.
#   - APK key password: DERUST_KEYSTORE_PASSWORD, else the macOS Keychain item
#     "tasmota-derust-keystore" (one-time setup in DEVELOPMENT.md); checked against the keystore.
#   - GPG: a test signature first (pinentry-mac fills the passphrase in from the Keychain
#     when "Save in Keychain" was ticked once).
#
# Steps, stopping at the first failure:
#   1. on main, clean tree, not behind origin/main, tag not taken, signing keys usable
#   2. cargo test, Trivy source scan (vulnerabilities + secrets, as security.yml),
#      actionlint, official Gradle wrapper
#   3. version bump in Cargo.toml + Cargo.lock, every target built (make), Trivy scan of the
#      release files (make package); the bump is undone if anything fails
#   4. signed commit "chore(release): vX.Y.Z" and signed tag vX.Y.Z
#   5. summary, confirmation, then `git push --atomic origin main vX.Y.Z`
#      (RELEASE_CONFIRM=yes skips the question; without a terminal it is required)
# DRY_RUN=1 runs 1-3, then puts Cargo.toml and Cargo.lock back and stops.
set -euo pipefail

cd "$(dirname "$0")/.."
# Never stop in a pager (git diff/log would wait for a key press).
export GIT_PAGER=cat PAGER=cat
forced=${1:-}
dry=${DRY_RUN:-0}

step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
die() { printf '\033[31mrelease: %s\033[0m\n' "$*" >&2; exit 1; }
semver='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'

# ------------------------------------------------------------------ 1. repository state
step "Repository"
[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || die "not on main"
[ -z "$(git status --porcelain)" ] || die "uncommitted changes: commit or stash them first"
git fetch --quiet --tags origin main
[ "$(git rev-list --count HEAD..origin/main)" = 0 ] || die "main is behind origin/main: pull first"

# Last release = highest version tag that is on origin and in main's history.
last=""
for t in $(git ls-remote --tags --refs origin 'v*' | sed 's|.*refs/tags/||' | sort -rV); do
    [[ "${t#v}" =~ $semver ]] || continue
    if git merge-base --is-ancestor "$t" HEAD 2>/dev/null; then last=$t; break; fi
done
[ -n "$last" ] || die "no version tag on origin: give the version (make release VERSION=0.1.0)"
current=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
echo "Last release: $last   Cargo.toml: $current"
[ "$(git rev-list --count "$last"..HEAD)" -gt 0 ] || die "nothing committed since $last"

# --------------------------------------------------------------- the version to release
if [ -n "$forced" ]; then
    forced=${forced#v}
    [[ "$forced" =~ $semver ]] || die "'$forced' is not a version like 1.2.3 or 1.2.3-rc1"
    next=$forced
    echo "Version given: $next"
else
    [[ "${last#v}" != *-* ]] || die "last tag $last is a pre-release: give the version"
    bump=none bad=()
    while IFS= read -r sha; do
        subject=$(git log -1 --format=%s "$sha")
        body=$(git log -1 --format=%b "$sha")
        if [[ "$subject" =~ ^([a-z]+)(\([^\)]+\))?(!)?:\  ]]; then
            type=${BASH_REMATCH[1]}
            if [ -n "${BASH_REMATCH[3]}" ] || grep -qE '^BREAKING[ -]CHANGE:' <<< "$body"; then
                die "breaking change in '$subject': majors are explicit (make release VERSION=x.0.0)"
            fi
            case "$type" in
                feat) bump=minor ;;
                fix|perf) [ "$bump" = minor ] || bump="patch" ;;
                docs|chore|ci|build|refactor|test|style|revert) ;;
                *) bad+=("$subject") ;;
            esac
        else
            bad+=("$subject")
        fi
        printf '  %-6s %s\n' "${type:-?}" "$subject"
        type=""
    done < <(git rev-list --reverse "$last"..HEAD)
    if [ ${#bad[@]} -gt 0 ]; then
        printf '  not Conventional Commits: %s\n' "${bad[@]}" >&2
        die "cannot pick the version by itself: give it (make release VERSION=x.y.z)"
    fi
    IFS=. read -r ma mi pa <<< "${last#v}"
    case "$bump" in
        minor) next="$ma.$((mi + 1)).0" ;;
        patch) next="$ma.$mi.$((pa + 1))" ;;
        none) die "only docs/chore/ci/... since $last: no release needed (or give the version)" ;;
    esac
    echo "Next version ($bump): $next"
fi
tag="v$next"
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists"
# Must be newer than the last release (sort -V puts 1.2.0-rc1 after 1.2.0, so compare the
# numbers; a pre-release of the next version is fine).
if [ "$(printf '%s\n%s\n' "${last#v}" "${next%%-*}" | sort -V | tail -1)" != "${next%%-*}" ] ||
    { [ "${next%%-*}" = "${last#v}" ] && [[ "$next" == *-* ]]; }; then
    die "$next is not newer than $last"
fi

# ------------------------------------------------------------------- signing keys
step "Signing keys"
gpgkey=$(git config user.signingkey || true)
[ -n "$gpgkey" ] || die "git user.signingkey is not set"
echo test | gpg --quiet --local-user "$gpgkey" --clearsign > /dev/null ||
    die "GPG cannot sign with $gpgkey (passphrase not available?)"
echo "GPG $gpgkey: ok"

ks=$HOME/.android/tasmota-derust-release.jks
if [ -f "$ks" ]; then
    if [ -z "${DERUST_KEYSTORE_PASSWORD:-}" ]; then
        DERUST_KEYSTORE_PASSWORD=$(security find-generic-password -s tasmota-derust-keystore -w 2>/dev/null) ||
            die "APK key password not found: set DERUST_KEYSTORE_PASSWORD or add it to the Keychain (DEVELOPMENT.md)"
    fi
    export DERUST_KEYSTORE_PASSWORD
    keytool="${JAVA_HOME:-/Applications/Android Studio.app/Contents/jbr/Contents/Home}/bin/keytool"
    "$keytool" -list -keystore "$ks" -storepass:env DERUST_KEYSTORE_PASSWORD > /dev/null 2>&1 ||
        die "the APK key password does not open $ks"
    echo "APK key: ok"
else
    echo "warning: $ks not found, the APK will be signed with the debug key" >&2
fi

# ------------------------------------------------------------------- 2. source checks
step "Tests"
cargo test --workspace --quiet

step "Trivy: dependencies and secrets"
command -v trivy >/dev/null || die "trivy not found (brew install trivy)"
skip=(--skip-dirs target --skip-dirs dist --skip-dirs android/app/build --skip-dirs android/build
      --skip-dirs android/.gradle)
trivy fs --quiet --scanners vuln,secret "${skip[@]}" --format table .
trivy fs --quiet --scanners vuln,secret "${skip[@]}" --severity HIGH,CRITICAL --ignore-unfixed \
    --exit-code 1 --format table . > /dev/null || die "Trivy found secrets or fixable HIGH/CRITICAL issues"

step "actionlint"
command -v actionlint >/dev/null || die "actionlint not found (brew install actionlint)"
actionlint

step "Gradle wrapper"
gv=$(sed -n 's/.*gradle-\([0-9.]*\)-bin.zip.*/\1/p' android/gradle/wrapper/gradle-wrapper.properties)
expected=$(curl -fsSL "https://services.gradle.org/distributions/gradle-${gv}-wrapper.jar.sha256")
actual=$(shasum -a 256 android/gradle/wrapper/gradle-wrapper.jar | cut -d' ' -f1)
[ "$expected" = "$actual" ] || die "gradle-wrapper.jar is not the official Gradle $gv wrapper"
echo "Gradle $gv wrapper: official"

# ------------------------------------------------------- 3. bump, build, scan the files
step "Version $current -> $next"
restore() { git checkout --quiet -- Cargo.toml Cargo.lock; }
trap 'restore; echo "release: stopped, Cargo.toml and Cargo.lock put back" >&2' ERR INT
sed -i '' "/^\[workspace.package\]/,/^\[/s/^version = \".*\"/version = \"$next\"/" Cargo.toml
cargo check --workspace --quiet
git diff --stat -- Cargo.toml Cargo.lock

step "Build every target"
make
step "Scan and pack the release files"
make package

if [ "$dry" = 1 ]; then
    trap - ERR INT
    restore
    step "Dry run: $tag would be released; Cargo.toml and Cargo.lock put back"
    exit 0
fi

# ------------------------------------------------------------------ 4. commit and tag
step "Signed commit and tag"
git commit --quiet -S -m "chore(release): $tag" -- Cargo.toml Cargo.lock
trap - ERR INT
git tag -s "$tag" -m "Tasmota Derust $next"
git log --show-signature -1 --format='%h %s' | tail -2
git tag -v "$tag" 2>&1 | grep -E 'Good signature|tag ' | head -2

# ------------------------------------------------------------------------ 5. push
step "Ready"
echo "Push main ($(git rev-list --count origin/main..HEAD) commits) and $tag to origin;"
echo "the Release workflow then builds and publishes it."
if [ "${RELEASE_CONFIRM:-}" != yes ]; then
    if [ -t 0 ]; then
        read -r -p "Push now? [y/N] " answer
    else
        answer=n
    fi
    if [[ "$answer" != [yY]* ]]; then
        echo "Not pushed. Later: git push --atomic origin main $tag"
        echo "To undo:  git tag -d $tag && git reset --hard HEAD~1"
        exit 1
    fi
fi
git push --atomic origin main "$tag"
echo "Pushed. Workflow: gh run list --workflow release.yml --limit 1"
