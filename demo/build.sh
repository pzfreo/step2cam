#!/bin/sh
# Builds the static site into site/: the page, step2cam and camlib as WebAssembly, and
# specify-core's released WebAssembly package.
#
#   demo/build.sh && python3 -m http.server -d site 8790
#
# Needs a Rust toolchain with the wasm32-unknown-unknown target, and wasm-pack.
set -eu
SPECIFY_VERSION=0.1.12
SPECIFY_SHA256=4e66f12f8a78bd0ec96011dc922fa58796f5b782f1c870e468c0b9740297f616
cd "$(dirname "$0")/.."
rust/fetch-camlib.sh

rm -rf site
mkdir -p site/licenses
wasm-pack build rust --release --target web --no-typescript --no-pack --out-dir ../site/step2cam
cp demo/index.html demo/app.js demo/stock.js demo/worker.js demo/style.css site/
cp tests/fixtures/plate.step tests/fixtures/synthetic/stepped-plate.step tests/fixtures/spindle.step site/
cp demo/tools.json site/tools.json
cp rust/capp/rulebase/rules.json site/rules.json

archive="specify-core-wasm-$SPECIFY_VERSION.tgz"
work=$(mktemp -d)
curl -fsSL -o "$work/$archive" "https://github.com/pzfreo/specify-core/releases/download/v$SPECIFY_VERSION/$archive"
echo "$SPECIFY_SHA256  $work/$archive" | shasum -a 256 -c -
tar -xzf "$work/$archive" -C "$work"
mkdir -p site/specify
cp "$work"/pkg/specify_core_wasm.js "$work"/pkg/specify_core_wasm_bg.wasm site/specify/
rm -rf "$work"

rm -f site/step2cam/.gitignore
cp LICENSE site/licenses/step2cam-LICENSE.txt
cp vendor/camlib/LICENSE site/licenses/camlib-LICENSE.txt
du -sh site site/step2cam/*.wasm site/specify/*.wasm
