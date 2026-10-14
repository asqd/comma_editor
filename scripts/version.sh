#!/bin/bash
# One version number for every build: the core (workspace.package.version), the macOS app
# (MARKETING_VERSION in apps/macos/project.yml) and the desktop app on Tauri (its Cargo.toml and
# package.json, which Tauri and the About window take it from). Setting one also lists the
# release in the Linux metadata.
#
#   scripts/version.sh             prints the version, or fails when the files disagree
#   scripts/version.sh 1.1.0       sets it everywhere
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

found() {
    sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/core (Cargo.toml): \1/p' Cargo.toml
    sed -n 's/^version = "\(.*\)"/desktop (apps\/desktop\/src-tauri\/Cargo.toml): \1/p' apps/desktop/src-tauri/Cargo.toml
    sed -n '1,5s/^  "version": "\(.*\)",/desktop (apps\/desktop\/package.json): \1/p' apps/desktop/package.json
    sed -n 's/^ *MARKETING_VERSION: "\(.*\)"/macOS (apps\/macos\/project.yml): \1/p' apps/macos/project.yml
}

if [[ $# -eq 0 ]]; then
    versions=$(found)
    distinct=$(echo "$versions" | sed 's/.*: //' | sort -u)
    if [[ $(echo "$distinct" | wc -l) -ne 1 ]]; then
        echo "error: the version differs between builds:" >&2
        echo "$versions" >&2
        echo "Set one with scripts/version.sh X.Y.Z" >&2
        exit 1
    fi
    echo "$distinct"
    exit 0
fi

VERSION="$1"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "error: a version is X.Y.Z, not $VERSION" >&2; exit 1; }
sed -i.bak '/^\[workspace.package\]/,/^\[/s/^version = ".*"/version = "'"$VERSION"'"/' Cargo.toml
sed -i.bak '1,/^version = /s/^version = ".*"/version = "'"$VERSION"'"/' apps/desktop/src-tauri/Cargo.toml
sed -i.bak '1,5s/^  "version": ".*",/  "version": "'"$VERSION"'",/' apps/desktop/package.json
# The lock file names the package's version at its top and for its root package.
sed -E -i.bak '1,10s/^( {2}| {6})"version": ".*",/\1"version": "'"$VERSION"'",/' apps/desktop/package-lock.json
sed -i.bak 's/^\( *MARKETING_VERSION: \)".*"/\1"'"$VERSION"'"/' apps/macos/project.yml
# Software centres on Linux list the releases, the newest first.
METAINFO=apps/desktop/packaging/linux/app.commaeditor.Desktop.metainfo.xml
if ! grep -q "<release version=\"$VERSION\"" "$METAINFO"; then
    sed -i.bak 's|^  <releases>$|  <releases>\
    <release version="'"$VERSION"'" date="'"$(date +%Y-%m-%d)"'"/>|' "$METAINFO"
    rm -f "$METAINFO.bak"
fi
rm -f Cargo.toml.bak apps/desktop/src-tauri/Cargo.toml.bak apps/desktop/package.json.bak apps/desktop/package-lock.json.bak apps/macos/project.yml.bak
# The lock files name the workspace's own crates with their versions; reading the metadata
# updates those and nothing else.
if command -v cargo >/dev/null; then
    cargo metadata --format-version 1 > /dev/null
    cargo metadata --manifest-path apps/desktop/src-tauri/Cargo.toml --format-version 1 > /dev/null
fi
echo "version $VERSION; raise CURRENT_PROJECT_VERSION in apps/macos/project.yml for the macOS app's build"
"$0"
