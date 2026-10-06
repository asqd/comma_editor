#!/bin/bash
# Builds the Rust core as a static library and generates its Swift bindings into
# apps/macos/CommaCore. Run by `make core` and by the Xcode pre-build phase.
#
# Xcode passes ARCHS: "arm64" for a debug build, "arm64 x86_64" for a release build. One
# library per architecture is built and they are joined into a universal one with lipo.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/macos/CommaCore"
export PATH="$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
ARCHITECTURES="${ARCHS:-$(uname -m)}"

cd "$ROOT"
GENERATED="$(mktemp -d)"
trap 'rm -rf "$GENERATED"' EXIT

libraries=()
for arch in $ARCHITECTURES; do
    case "$arch" in
        arm64) target=aarch64-apple-darwin ;;
        x86_64) target=x86_64-apple-darwin ;;
        *) echo "error: unsupported architecture $arch" >&2; exit 1 ;;
    esac
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "error: Rust target $target is missing; run: rustup target add $target" >&2
        exit 1
    fi
    cargo build --release --package comma-ffi --lib --target "$target"
    libraries+=("target/$target/release/libcomma_ffi.a")
done
lipo -create "${libraries[@]}" -output "$GENERATED/libcomma_ffi.a"

# The bindings come from a library built for this Mac.
cargo build --release --package comma-ffi --lib
cargo build --release --package uniffi-bindgen-swift
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
install_if_changed "$GENERATED/libcomma_ffi.a" "$OUT/lib/libcomma_ffi.a"
