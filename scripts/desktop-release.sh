#!/bin/bash
# Builds the installers of the desktop app on Tauri for the system it runs on, into build/desktop
# with a .sha256 beside each. No signing certificates: the DMG is signed ad hoc, as the macOS
# app's is; the Windows installer is not signed. See docs/RELEASE.md.
#
#   scripts/desktop-release.sh          macOS: universal DMG of CommaEditor W
#                                       Windows: NSIS installer for x64
#                                       Linux: AppImage and .deb for this machine's
#                                       architecture, x86_64 or aarch64
#   TARGET=aarch64-pc-windows-msvc scripts/desktop-release.sh
#                                       the Windows installer for ARM64, built on x64
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$("$ROOT/scripts/version.sh")"
OUT="$ROOT/build/desktop"
mkdir -p "$OUT"

case "$(uname -s)" in
    Darwin) system=macos; target="${TARGET:-universal-apple-darwin}"; bundles=dmg ;;
    MINGW* | MSYS* | CYGWIN*) system=windows; target="${TARGET:-x86_64-pc-windows-msvc}"; bundles=nsis ;;
    *) system=linux; target="${TARGET:-$(uname -m)-unknown-linux-gnu}"; bundles=appimage,deb ;;
esac

cd "$ROOT/apps/desktop"
[[ -d node_modules ]] || npm ci
npx tauri build --target "$target" --bundles "$bundles"
BUNDLE="$ROOT/apps/desktop/src-tauri/target/$target/release/bundle"

# Names say what each file is for, as on the download page.
copy() {
    cp "$1" "$OUT/$2"
    echo "$OUT/$2"
}
built=()
case "$system" in
    macos)
        built+=("$(copy "$BUNDLE"/dmg/*.dmg "CommaEditor-W-$VERSION-macos-universal.dmg")")
        ;;
    windows)
        arch=x64
        [[ "$target" == aarch64-* ]] && arch=arm64
        built+=("$(copy "$BUNDLE"/nsis/*-setup.exe "CommaEditor-$VERSION-windows-$arch-setup.exe")")
        ;;
    linux)
        # AppImages name the architecture as the kernel does, packages as Debian does.
        arch="${target%%-*}"
        deb=amd64
        [[ "$arch" == aarch64 ]] && deb=arm64
        built+=("$(copy "$BUNDLE"/appimage/*.AppImage "CommaEditor-$VERSION-linux-$arch.AppImage")")
        built+=("$(copy "$BUNDLE"/deb/*.deb "commaeditor_${VERSION}_$deb.deb")")
        ;;
esac

for file in "${built[@]}"; do
    name="$(basename "$file")"
    if command -v sha256sum >/dev/null; then
        (cd "$OUT" && sha256sum "$name" > "$name.sha256")
    else
        (cd "$OUT" && shasum -a 256 "$name" > "$name.sha256")
    fi
    echo "built $name: $(cut -d' ' -f1 "$file.sha256")"
done
