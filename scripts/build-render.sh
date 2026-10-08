#!/bin/sh
# Builds the plant's renderer for the browser and drops the bundle where
# `harness-view` serves it from: crates/view/static/render/{render.js,render_bg.wasm}.
#
# Needs a rustup toolchain with the wasm32 target (`rustup target add
# wasm32-unknown-unknown`) and wasm-pack (`brew install wasm-pack`), which
# fetches a matching wasm-bindgen itself and runs wasm-opt when it is on PATH.
#
# Usage: scripts/build-render.sh [wasm-dev|wasm-release]
#   wasm-dev      (default) fast rebuild, larger bundle — for working on the plant
#   wasm-release  minutes of LTO and wasm-opt, the small bundle — before sharing
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
profile=${1:-wasm-dev}
out="$root/crates/view/static/render"

# A Homebrew `cargo` on PATH has no wasm32 standard library; the rustup
# toolchain does. Put it first for this build only.
if command -v rustup >/dev/null 2>&1; then
  toolchain=$(dirname "$(dirname "$(rustup which rustc)")")
  PATH="$toolchain/bin:$PATH"
  export PATH
  # rustup's rust-lld on macOS looks for libLLVM.dylib next to itself and
  # finds nothing; the library sits two levels up. The fallback path is the
  # one dyld consults last, so it changes nothing when the lookup succeeds.
  DYLD_FALLBACK_LIBRARY_PATH="$toolchain/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
  export DYLD_FALLBACK_LIBRARY_PATH
fi
echo "building with $(cargo --version) for wasm32-unknown-unknown, profile $profile"

mkdir -p "$out"
mode="--profile $profile"
# shellcheck disable=SC2086 # $mode is two words on purpose.
wasm-pack build crates/view-render \
  --target web \
  --no-pack \
  --no-typescript \
  --no-opt \
  --out-dir "$out" \
  --out-name render \
  $mode
rm -f "$out/.gitignore"

# Shrinks the module by a third. wasm-pack's own call is disabled in
# Cargo.toml because it refuses the bulk-memory instructions rustc emits.
if [ "$profile" = wasm-release ] && command -v wasm-opt >/dev/null 2>&1; then
  echo "wasm-opt -Oz"
  wasm-opt -Oz --all-features "$out/render_bg.wasm" -o "$out/render_bg.opt.wasm"
  mv "$out/render_bg.opt.wasm" "$out/render_bg.wasm"
fi
ls -la "$out"
