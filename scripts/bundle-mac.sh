#!/bin/sh
# Builds target/Footplate.app from the release binary, for macOS only: the
# icon's sizes are rendered with sips and packed with iconutil, which no
# other system has. Run it after `cargo build --release`. There is no
# installer: the bundle is for the machine that built it, signed ad hoc and
# not notarized. With two arguments another binary is bundled elsewhere,
# which the release script uses for each darwin target:
# `scripts/bundle-mac.sh <binary> <app>`.
#
# FOOTPLATE_RELEASE_SIGN_IDENTITY, the name of a "Developer ID Application"
# identity of the keychain, signs the bundle with it instead, with the
# hardened runtime and a secure timestamp, which Apple's notary service asks
# of a signature. Notarizing is the release script's, which asks for a
# notary profile beside the identity (README.md, "Building").
set -e

if [ "$(uname -s)" != Darwin ]; then
    echo "bundle-mac.sh: a macOS bundle is built on macOS (sips, iconutil)" >&2
    exit 1
fi

if [ $# -ne 0 ] && [ $# -ne 2 ]; then
    echo "bundle-mac.sh: usage: scripts/bundle-mac.sh [<binary> <app>]" >&2
    exit 2
fi

cd "$(dirname "$0")/.."
binary=${1:-target/release/footplate}
icon=assets/icon.png
app=${2:-target/Footplate.app}

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

# Signed as a bundle, the signature carries the bundle's id, which the
# system files its notifications under, where the linker signed the binary
# alone, under a name of its own making. --force replaces that signature.
identity=${FOOTPLATE_RELEASE_SIGN_IDENTITY:-}
if [ -n "$identity" ]; then
    # The notary service refuses a signature without the hardened runtime or
    # without a secure timestamp, so both are asked for; and the signature
    # is checked here, so that one that does not hold fails here and not
    # after the wait for Apple.
    codesign --force --options runtime --timestamp --sign "$identity" "$app"
    codesign --verify --strict "$app"
else
    # Ad hoc, with no identity: a builder's or a contributor's build.
    codesign --force --sign - "$app"
fi

echo "$app"
