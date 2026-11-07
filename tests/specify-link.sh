#!/bin/sh
# Runs specify-link.mjs against the specify-core release demo/build.sh ships.
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^SPECIFY_VERSION=//p' demo/build.sh)
sha=$(sed -n 's/^SPECIFY_SHA256=//p' demo/build.sh)
archive="specify-core-wasm-$version.tgz"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL -o "$work/$archive" "https://github.com/pzfreo/specify-core/releases/download/v$version/$archive"
echo "$sha  $work/$archive" | shasum -a 256 -c -
tar -xzf "$work/$archive" -C "$work"
node tests/specify-link.mjs "$work/pkg"
