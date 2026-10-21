#!/bin/sh
# Wraps a built editor into "Lumen Edit.app" (agents in Contents/Resources).
#   scripts/bundle-macos.sh [editor binary] [output dir]
# Defaults: target/release/lumen-edit, target. The bundle is not signed:
# after downloading run `xattr -dr com.apple.quarantine "Lumen Edit.app"`.
set -e
cd "$(dirname "$0")/.."
binary="${1:-target/release/lumen-edit}"
out="${2:-target}"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app="$out/Lumen Edit.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/agents"
cp "$binary" "$app/Contents/MacOS/lumen-edit"
cp target/agents/lumen-agent-* "$app/Contents/Resources/agents/" 2>/dev/null || true

# Icon from the SVG, when the tools for it are at hand.
if command -v rsvg-convert >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
    set_dir="$out/lumen-edit.iconset"
    mkdir -p "$set_dir"
    for size in 16 32 128 256 512; do
        rsvg-convert -w "$size" -h "$size" assets/lumen-edit.svg -o "$set_dir/icon_${size}x${size}.png"
        rsvg-convert -w "$((size * 2))" -h "$((size * 2))" assets/lumen-edit.svg -o "$set_dir/icon_${size}x${size}@2x.png"
    done
    iconutil -c icns "$set_dir" -o "$app/Contents/Resources/lumen-edit.icns"
    rm -rf "$set_dir"
fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Lumen Edit</string>
    <key>CFBundleDisplayName</key><string>Lumen Edit</string>
    <key>CFBundleIdentifier</key><string>dev.lumen-edit</string>
    <key>CFBundleExecutable</key><string>lumen-edit</string>
    <key>CFBundleIconFile</key><string>lumen-edit</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
echo "$app"
