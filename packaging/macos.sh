#!/usr/bin/env bash
# Makes the disk image with the app from a release build.
# Usage: packaging/macos.sh <version> <out folder>
set -euo pipefail
version=$1
out=$2
work=$(mktemp -d)
app="$work/image/Apeiron.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$out"
cp target/release/apeiron-desktop target/release/apeiron "$app/Contents/MacOS/"

python3 packaging/make_icons.py "$work/icons"
set_dir="$work/apeiron.iconset"
mkdir "$set_dir"
for size in 16 32 128 256 512; do
  cp "$work/icons/icon-$size.png" "$set_dir/icon_${size}x${size}.png"
done
for size in 16 32 128 256 512; do
  double=$((size * 2))
  if [ -f "$work/icons/icon-$double.png" ]; then
    cp "$work/icons/icon-$double.png" "$set_dir/icon_${size}x${size}@2x.png"
  fi
done
iconutil -c icns "$set_dir" -o "$app/Contents/Resources/apeiron.icns"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Apeiron</string>
  <key>CFBundleIdentifier</key><string>org.apeiron.desktop</string>
  <key>CFBundleExecutable</key><string>apeiron-desktop</string>
  <key>CFBundleIconFile</key><string>apeiron</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
ln -s /Applications "$work/image/Applications"
hdiutil create -volname "Apeiron $version" -srcfolder "$work/image" -format UDZO \
  "$out/Apeiron-$version-macos-$(uname -m).dmg"
