#!/bin/sh
# Install the unpacked Linux archive for the current user (no root needed).
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
dest="$data/fluxdm-app"
mkdir -p "$dest" "$data/applications" "$data/icons/hicolor/512x512/apps" "$HOME/.local/bin"
rm -rf "$dest/extension"
cp -R "$here/fluxdm" "$here/fluxdm-host" "$here/extension" "$dest/"
cp "$here/fluxdm.png" "$data/icons/hicolor/512x512/apps/"
sed "s|^Exec=.*|Exec=\"$dest/fluxdm\"|" "$here/fluxdm.desktop" > "$data/applications/fluxdm.desktop"
ln -sf "$dest/fluxdm" "$HOME/.local/bin/fluxdm"
"$dest/fluxdm-host" install || echo "Open your browser once, then run: $dest/fluxdm-host install"
echo "Installed. Load the browser extension from: $dest/extension"
