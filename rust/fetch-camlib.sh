#!/bin/sh
# camlib at the pinned commit, with the patch that lets it compile for wasm32. It goes when
# camlib has the change.
set -eu
COMMIT=d05c0df8d74d6b766e1460da4c70010005e676f8
cd "$(dirname "$0")/.."
if [ "$(git -C vendor/camlib rev-parse HEAD 2>/dev/null)" = "$COMMIT" ]; then
    exit 0
fi
rm -rf vendor/camlib
mkdir -p vendor
git init -q vendor/camlib
git -C vendor/camlib fetch -q --depth 1 https://github.com/stevecad/camlib "$COMMIT"
git -C vendor/camlib checkout -q FETCH_HEAD
git -C vendor/camlib apply "$PWD/rust/camlib-wasm32.patch"
