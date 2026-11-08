#!/bin/sh
# The site build's tools for Linux x86_64, through GitHub's API (gh, with the workflow's token):
# GitHub's plain release links have returned 504 for hours at a time, and wasm-pack's npm installer
# and its own downloads of wasm-bindgen and wasm-opt use them. wasm-pack uses the wasm-bindgen and
# wasm-opt it finds on PATH. Each archive is checked against its SHA-256 (the publishers' own, for
# wasm-bindgen and binaryen). wasm-bindgen must match rust/Cargo.lock's wasm-bindgen.
set -eu
dir=${RUNNER_TEMP:-$(mktemp -d)}/wasm-tools
mkdir -p "$dir"
get() { # repo, tag, asset, sha256, the folder in the archive holding the binaries
    gh release download "$2" -R "$1" -p "$3" -D "$dir" --clobber
    echo "$4  $dir/$3" | sha256sum -c -
    tar -xzf "$dir/$3" -C "$dir"
    echo "$dir/$5" >> "$GITHUB_PATH"
}
get wasm-bindgen/wasm-pack v0.15.1 wasm-pack-v0.15.1-x86_64-unknown-linux-musl.tar.gz \
    410101009739b9d01c7d5affaaad6e9c9a24b4cba6791a4805db8e89b9bf7af2 wasm-pack-v0.15.1-x86_64-unknown-linux-musl
get wasm-bindgen/wasm-bindgen 0.2.130 wasm-bindgen-0.2.130-x86_64-unknown-linux-musl.tar.gz \
    9687f3bb551b9f66f37ed32a40d7913a00f7962db653054e56857bc306c3227f wasm-bindgen-0.2.130-x86_64-unknown-linux-musl
get WebAssembly/binaryen version_130 binaryen-version_130-x86_64-linux.tar.gz \
    0a18362361ad05465118cd8eeb72edaeec89de6894bc283576ef4e07aa3babcc binaryen-version_130/bin
