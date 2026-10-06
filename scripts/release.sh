#!/bin/bash
# Builds a universal (Apple Silicon + Intel) release of CommaEditor and packs it into a DMG.
#
#   scripts/release.sh                      local build, ad-hoc signed: for testing only
#   DEVELOPMENT_TEAM=ABCDE12345 \
#   NOTARY_PROFILE=commaeditor \
#   scripts/release.sh                      Developer ID signed, notarized and stapled
#
# Signing needs a "Developer ID Application" certificate in the keychain. Notarization needs a
# notarytool profile, created once with:
#   xcrun notarytool store-credentials commaeditor --apple-id you@example.com --team-id ABCDE12345
# See docs/RELEASE.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/build/release"
PROJECT="$ROOT/apps/macos/CommaEditor.xcodeproj"
TEAM="${DEVELOPMENT_TEAM:-}"
NOTARY_PROFILE="${NOTARY_PROFILE:-}"

cd "$ROOT"
rm -rf "$BUILD"
mkdir -p "$BUILD"
make xcodeproj

signing=(CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY=-)
if [[ -n "$TEAM" ]]; then
    signing=(
        CODE_SIGN_STYLE=Manual
        "CODE_SIGN_IDENTITY=Developer ID Application"
        "DEVELOPMENT_TEAM=$TEAM"
        "OTHER_CODE_SIGN_FLAGS=--timestamp"
    )
else
    echo "note: DEVELOPMENT_TEAM is not set, so the build is ad-hoc signed and will not open on other Macs"
fi

xcodebuild -project "$PROJECT" -scheme CommaEditor -configuration Release \
    -destination "generic/platform=macOS" \
    -derivedDataPath "$BUILD/DerivedData" \
    -archivePath "$BUILD/CommaEditor.xcarchive" \
    "${signing[@]}" \
    archive | grep -E "error:|warning:|ARCHIVE (SUCCEEDED|FAILED)" || true
APP="$BUILD/CommaEditor.xcarchive/Products/Applications/CommaEditor.app"
[[ -d "$APP" ]] || { echo "error: the archive has no app" >&2; exit 1; }

if [[ -n "$TEAM" ]]; then
    cat > "$BUILD/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>method</key><string>developer-id</string>
    <key>teamID</key><string>$TEAM</string>
    <key>signingStyle</key><string>manual</string>
    <key>signingCertificate</key><string>Developer ID Application</string>
</dict>
</plist>
PLIST
    xcodebuild -exportArchive -archivePath "$BUILD/CommaEditor.xcarchive" \
        -exportOptionsPlist "$BUILD/ExportOptions.plist" -exportPath "$BUILD/export"
    APP="$BUILD/export/CommaEditor.app"
fi

echo "== checks"
lipo -archs "$APP/Contents/MacOS/CommaEditor"
codesign --verify --deep --strict --verbose=1 "$APP"
VERSION=$(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" "$APP/Contents/Info.plist")

echo "== disk image"
STAGING="$BUILD/dmg"
mkdir -p "$STAGING"
cp -R "$APP" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
DMG="$BUILD/CommaEditor-$VERSION.dmg"
hdiutil create -volname "CommaEditor $VERSION" -srcfolder "$STAGING" -ov -format UDZO "$DMG" >/dev/null

if [[ -n "$TEAM" ]]; then
    codesign --sign "Developer ID Application" --timestamp "$DMG"
    if [[ -n "$NOTARY_PROFILE" ]]; then
        echo "== notarization (takes a few minutes)"
        xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
        xcrun stapler staple "$DMG"
        spctl --assess --type open --context context:primary-signature --verbose "$DMG"
    else
        echo "note: NOTARY_PROFILE is not set, so the DMG is signed but not notarized"
    fi
fi
echo "== done: $DMG"
