#!/bin/bash
# Builds the Rust core as a static library and generates its Swift bindings into
# apps/macos/CommaCore. Run by `make core` and by the Xcode pre-build phase.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/macos/CommaCore"
export PATH="$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"

cd "$ROOT"
cargo build --release --package comma-ffi --lib
cargo build --release --package uniffi-bindgen-swift

GENERATED="$(mktemp -d)"
trap 'rm -rf "$GENERATED"' EXIT
target/release/uniffi-bindgen-swift \
    --swift-sources --headers --modulemap \
    --module-name comma_ffiFFI --modulemap-filename module.modulemap \
    target/release/libcomma_ffi.dylib "$GENERATED"

mkdir -p "$OUT/include" "$OUT/lib" "$OUT/Sources"

# Copy only what changed, so Xcode does not recompile the bindings on every build.
install_if_changed() {
    cmp -s "$1" "$2" || cp "$1" "$2"
}
install_if_changed "$GENERATED/comma_ffi.swift" "$OUT/Sources/comma_ffi.swift"
install_if_changed "$GENERATED/comma_ffiFFI.h" "$OUT/include/comma_ffiFFI.h"
install_if_changed "$GENERATED/module.modulemap" "$OUT/include/module.modulemap"
install_if_changed target/release/libcomma_ffi.a "$OUT/lib/libcomma_ffi.a"
