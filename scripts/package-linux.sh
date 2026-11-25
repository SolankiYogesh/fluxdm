#!/bin/sh
# Build a .deb and a .tar.gz for this machine's architecture into dist/.
set -eu
cd "$(dirname "$0")/.."
[ "${SKIP_BUILD:-}" = 1 ] || cargo build --release --locked
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(dpkg --print-architecture)
work=target/package/linux
rm -rf "$work"
mkdir -p dist

# Files installed by both packages, laid out under one folder.
payload="$work/FluxDM-$version-linux-$arch"
mkdir -p "$payload"
cp target/release/fluxdm target/release/fluxdm-host "$payload/"
cp assets/icon-512.png "$payload/fluxdm.png"
cp packaging/fluxdm.desktop packaging/install.sh "$payload/"
cp crates/app/assets/LICENSE-*.txt "$payload/"
sh scripts/package-extension.sh "$payload/extension"
chmod 755 "$payload/install.sh"
tar -C "$work" -czf "dist/FluxDM-$version-linux-$arch.tar.gz" "$(basename "$payload")"

deb="$work/deb"
mkdir -p "$deb/DEBIAN" "$deb/usr/lib/fluxdm" "$deb/usr/bin" \
    "$deb/usr/share/applications" "$deb/usr/share/icons/hicolor/512x512/apps" "$deb/usr/share/doc/fluxdm"
cp "$payload/fluxdm" "$payload/fluxdm-host" "$deb/usr/lib/fluxdm/"
cp -R "$payload/extension" "$deb/usr/lib/fluxdm/"
cp "$payload"/LICENSE-*.txt "$deb/usr/share/doc/fluxdm/"
ln -s ../lib/fluxdm/fluxdm "$deb/usr/bin/fluxdm"
ln -s ../lib/fluxdm/fluxdm-host "$deb/usr/bin/fluxdm-host"
cp packaging/fluxdm.desktop "$deb/usr/share/applications/"
cp assets/icon-512.png "$deb/usr/share/icons/hicolor/512x512/apps/fluxdm.png"
cat > "$deb/DEBIAN/control" <<CONTROL
Package: fluxdm
Version: $version
Architecture: $arch
Maintainer: FluxDM <noreply@localhost>
Section: net
Priority: optional
Depends: libc6, libxkbcommon0, libxkbcommon-x11-0, libgl1, libegl1
Recommends: xdg-desktop-portal, ffmpeg
Description: Segmented download manager
 Downloads files over several connections, with a browser extension.
 After installing, run "fluxdm-host install" once as your own user.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "dist/FluxDM-$version-linux-$arch.deb"
ls -lh dist
