#!/usr/bin/env bash
# Short Markdown changelog from the Conventional Commits between two refs.
#   scripts/changelog.sh <from> <to>        e.g. scripts/changelog.sh v0.1.0 v0.1.1
#
# feat -> "New", fix/perf -> "Fixes", docs/build/ci/refactor/test/style/revert and commits
# that are not Conventional Commits -> "Other". Release commits (chore(release)) and other
# chores are left out. Only the subject line is used, scopes are kept as a prefix.
set -euo pipefail

from=$1 to=$2
export GIT_PAGER=cat
new=() fixes=() other=()
while IFS= read -r subject; do
    if [[ "$subject" =~ ^([a-z]+)(\(([^\)]+)\))?!?:\ (.*)$ ]]; then
        type=${BASH_REMATCH[1]} scope=${BASH_REMATCH[3]} text=${BASH_REMATCH[4]}
    else
        type=other scope="" text=$subject
    fi
    text="$(tr '[:lower:]' '[:upper:]' <<< "${text:0:1}")${text:1}"
    [ -n "$scope" ] && text="$scope: $text"
    case "$type" in
        feat) new+=("$text") ;;
        fix|perf) fixes+=("$text") ;;
        chore) ;;
        *) other+=("$text") ;;
    esac
done < <(git log --reverse --no-merges --format=%s "$from..$to")

section() { # section <title> <items...>
    local title=$1; shift
    [ $# -gt 0 ] || return 0
    printf '### %s\n\n' "$title"
    printf -- '- %s\n' "$@"
    printf '\n'
}
section "New" ${new[@]+"${new[@]}"}
section "Fixes" ${fixes[@]+"${fixes[@]}"}
section "Other" ${other[@]+"${other[@]}"}
