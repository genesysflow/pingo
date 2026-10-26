#!/usr/bin/env sh
# Builds crates/pingo-denoise (DeepFilterNet3) for the browser noise suppressor
# and copies it to public/denoise/. With --optional, a missing Rust toolchain or
# wasm32 target only warns: calls then fall back to the browser's suppressor.
set -eu
# The desktop app suppresses noise natively (pingo-media); vite.config.ts also
# leaves the browser build out of its bundle.
if [ -n "${TAURI_ENV_PLATFORM:-}" ]; then exit 0; fi
web=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$web/../.." && pwd)
out="$web/public/denoise"
build() {
  RUSTFLAGS="-C target-feature=+simd128" cargo build --quiet \
    --manifest-path "$root/Cargo.toml" -p pingo-denoise \
    --profile denoise-wasm --target wasm32-unknown-unknown --target-dir "$root/target/wasm" &&
    mkdir -p "$out" &&
    cp "$root/target/wasm/wasm32-unknown-unknown/denoise-wasm/pingo_denoise.wasm" "$out/pingo-denoise.wasm"
}
if ! build; then
  if [ "${1:-}" = "--optional" ]; then
    echo "warning: could not build the DeepFilterNet WASM (needs: rustup target add wasm32-unknown-unknown)." >&2
    echo "warning: browser calls will use the browser's built-in noise suppression." >&2
    exit 0
  fi
  exit 1
fi
