#!/bin/sh
# Builds target/Footplate.app from the release binary, for macOS only: the
# icon's sizes are rendered with sips and packed with iconutil, which no
# other system has. Run it after `cargo build --release`. Nothing is signed
# or notarized, and there is no installer: the bundle is for the machine
# that built it.
set -e

if [ "$(uname -s)" != Darwin ]; then
    echo "bundle-mac.sh: a macOS bundle is built on macOS (sips, iconutil)" >&2
    exit 1
fi

cd "$(dirname "$0")/.."
binary=target/release/footplate
icon=assets/icon.png
app=target/Footplate.app

if [ ! -x "$binary" ]; then
    echo "bundle-mac.sh: no $binary: run cargo build --release first" >&2
    exit 1
fi

# From nothing each time, so no file of an earlier bundle is left in it.
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/footplate"

# iconutil goes by the directory's ending and by these names: each size in
# points, and again at twice the pixels.
iconset_root=$(mktemp -d)
trap 'rm -rf "$iconset_root"' 0
trap 'exit 1' HUP INT TERM
iconset=$iconset_root/footplate.iconset
mkdir "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$icon" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z "$((size * 2))" "$((size * 2))" "$icon" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns --output "$app/Contents/Resources/footplate.icns" "$iconset"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Footplate</string>
    <key>CFBundleIdentifier</key>
    <string>dev.yardr.footplate</string>
    <key>CFBundleExecutable</key>
    <string>footplate</string>
    <key>CFBundleIconFile</key>
    <string>footplate</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

echo "$app"
