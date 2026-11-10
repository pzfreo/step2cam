#!/bin/sh
# camlib at the pinned commit: it is not published.
set -eu
COMMIT=c6383a5cc9cb64430551867e538af53fbce91e0c
cd "$(dirname "$0")/.."
if [ "$(git -C vendor/camlib rev-parse HEAD 2>/dev/null)" = "$COMMIT" ]; then
    exit 0
fi
rm -rf vendor/camlib
mkdir -p vendor
git init -q vendor/camlib
git -C vendor/camlib fetch -q --depth 1 https://github.com/stevecad/camlib "$COMMIT"
git -C vendor/camlib checkout -q FETCH_HEAD
