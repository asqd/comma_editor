#!/bin/bash
# Renders design/make-icon.swift and fills the app icon set with every size macOS needs, then the
# desktop app's icons: plain for Windows and Linux, with a W badge for CommaEditor W on macOS.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SET="$ROOT/apps/macos/CommaEditor/Resources/Assets.xcassets/AppIcon.appiconset"
mkdir -p "$SET"
swift "$ROOT/design/make-icon.swift" "$ROOT/design/icon_1024.png"
for size in 16 32 128 256 512; do
    sips -z $size $size "$ROOT/design/icon_1024.png" --out "$SET/icon_${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$ROOT/design/icon_1024.png" --out "$SET/icon_${size}@2x.png" >/dev/null
done
cat > "$SET/Contents.json" <<'JSON'
{
  "images" : [
    { "idiom" : "mac", "scale" : "1x", "size" : "16x16", "filename" : "icon_16.png" },
    { "idiom" : "mac", "scale" : "2x", "size" : "16x16", "filename" : "icon_16@2x.png" },
    { "idiom" : "mac", "scale" : "1x", "size" : "32x32", "filename" : "icon_32.png" },
    { "idiom" : "mac", "scale" : "2x", "size" : "32x32", "filename" : "icon_32@2x.png" },
    { "idiom" : "mac", "scale" : "1x", "size" : "128x128", "filename" : "icon_128.png" },
    { "idiom" : "mac", "scale" : "2x", "size" : "128x128", "filename" : "icon_128@2x.png" },
    { "idiom" : "mac", "scale" : "1x", "size" : "256x256", "filename" : "icon_256.png" },
    { "idiom" : "mac", "scale" : "2x", "size" : "256x256", "filename" : "icon_256@2x.png" },
    { "idiom" : "mac", "scale" : "1x", "size" : "512x512", "filename" : "icon_512.png" },
    { "idiom" : "mac", "scale" : "2x", "size" : "512x512", "filename" : "icon_512@2x.png" }
  ],
  "info" : { "author" : "xcode", "version" : 1 }
}
JSON
printf '{\n  "info" : { "author" : "xcode", "version" : 1 }\n}\n' > "$SET/../Contents.json"
echo "icon set written to $SET"

swift "$ROOT/design/make-icon.swift" "$ROOT/design/icon_w_1024.png" W
DESKTOP="$ROOT/apps/desktop"
ICONS="$DESKTOP/src-tauri/icons"
if [ -x "$DESKTOP/node_modules/.bin/tauri" ]; then
    TEMP="$(mktemp -d)"
    trap 'rm -rf "$TEMP"' EXIT
    (cd "$DESKTOP" && npx tauri icon "$ROOT/design/icon_1024.png" -o "$TEMP/plain" >/dev/null 2>&1)
    (cd "$DESKTOP" && npx tauri icon "$ROOT/design/icon_w_1024.png" -o "$TEMP/w" >/dev/null 2>&1)
    rm -rf "$TEMP/plain/android" "$TEMP/plain/ios"
    cp "$TEMP/plain/"* "$ICONS/"
    mkdir -p "$ICONS/macos"
    for file in 32x32.png 128x128.png 128x128@2x.png icon.png icon.icns; do
        cp "$TEMP/w/$file" "$ICONS/macos/$file"
    done
    echo "desktop icons written to $ICONS"
else
    echo "skipped the desktop icons: run npm ci in apps/desktop first"
fi
