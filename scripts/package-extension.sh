#!/bin/sh
# Copy only the files the browser extension needs into $1.
set -eu
dest=$1
src="$(cd "$(dirname "$0")/../extension" && pwd)"
rm -rf "$dest"
mkdir -p "$dest"
for f in manifest.json background.js content.js popup.js links.js rules.js popup.html links.html theme.css icons; do
    cp -R "$src/$f" "$dest/"
done
