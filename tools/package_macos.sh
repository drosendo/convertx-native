#!/usr/bin/env bash
# Builds a double-clickable macOS app bundle at dist/ConvertX.app
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="${HOME}/.cargo/bin:${PATH}"

APP_NAME="ConvertX"
BUNDLE_ID="com.rosendolabs.convertx"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
DIST="dist"
APP="$DIST/$APP_NAME.app"

echo "Building release binary…"
cargo build --release

echo "Rendering the icon…"
mkdir -p build
"target/release/convertx" --to png --via native-svg --out build build/icon.svg >/dev/null

ICON_PNG="$(ls -t build/icon*.png | head -1)"
ICONSET="build/$APP_NAME.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$ICON_PNG" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$ICON_PNG" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "build/$APP_NAME.icns"

echo "Assembling ${APP}…"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/convertx "$APP/Contents/MacOS/convertx"
cp "build/$APP_NAME.icns" "$APP/Contents/Resources/$APP_NAME.icns"
cp LICENSE CREDITS.md README.md "$APP/Contents/Resources/"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>            <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>     <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>      <string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key>      <string>convertx</string>
    <key>CFBundleIconFile</key>        <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>     <string>APPL</string>
    <key>CFBundleShortVersionString</key> <string>$VERSION</string>
    <key>CFBundleVersion</key>         <string>$VERSION</string>
    <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
    <key>LSMinimumSystemVersion</key>  <string>11.0</string>
    <key>LSApplicationCategoryType</key> <string>public.app-category.utilities</string>
    <key>NSHighResolutionCapable</key> <true/>
    <key>NSHumanReadableCopyright</key>
    <string>AGPL-3.0. Derived from ConvertX by C4illin.</string>
</dict>
</plist>
PLIST

# Ad-hoc signature: enough for local use without a developer certificate.
# Cargo lives outside PATH when this script is run from Finder/CI.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || \
  echo "note: ad-hoc signing skipped"

echo "Done: ${APP}"
echo "Open it with: open $APP"
