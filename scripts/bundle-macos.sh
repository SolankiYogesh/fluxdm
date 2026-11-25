#!/bin/sh
# Create FluxDM.app.  usage: bundle-macos.sh <folder with binaries> <path/FluxDM.app>
set -eu
bin=$1
app=$2
root="$(cd "$(dirname "$0")/.." && pwd)"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin/fluxdm" "$bin/fluxdm-host" "$app/Contents/MacOS/"
cp "$root/assets/icon.icns" "$app/Contents/Resources/"
cp "$root"/crates/app/assets/LICENSE-*.txt "$app/Contents/Resources/"
sh "$root/scripts/package-extension.sh" "$app/Contents/Resources/extension"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>CFBundleName</key><string>FluxDM</string>
    <key>CFBundleIdentifier</key><string>local.fluxdm.app</string>
    <key>CFBundleExecutable</key><string>fluxdm</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --deep --sign - "$app"
