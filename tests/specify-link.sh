#!/bin/sh
# Runs specify-link.mjs against the specify-core release demo/build.sh ships.
set -eu
# specify-core's release, through GitHub's API when gh is signed in (the plain release link has
# returned 504 for hours at a time), else the plain link, retried.
fetch_specify() { # version, archive, directory
    if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
        gh release download "v$1" -R pzfreo/specify-core -p "$2" -D "$3" && return 0
    fi
    curl -fsSL --retry 5 --retry-all-errors --retry-delay 10 -o "$3/$2" "https://github.com/pzfreo/specify-core/releases/download/v$1/$2"
}
cd "$(dirname "$0")/.."
version=$(sed -n 's/^SPECIFY_VERSION=//p' demo/build.sh)
sha=$(sed -n 's/^SPECIFY_SHA256=//p' demo/build.sh)
archive="specify-core-wasm-$version.tgz"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fetch_specify "$version" "$archive" "$work"
echo "$sha  $work/$archive" | shasum -a 256 -c -
tar -xzf "$work/$archive" -C "$work"
node tests/specify-link.mjs "$work/pkg"
