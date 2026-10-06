#!/bin/bash
# Renders design/make-icon.swift and fills the app icon set with every size macOS needs.
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
