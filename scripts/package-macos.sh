#!/bin/sh
# Build a universal (Apple silicon and Intel) disk image into dist/.
set -eu
cd "$(dirname "$0")/.."
targets="aarch64-apple-darwin x86_64-apple-darwin"
for t in $targets; do
    cargo build --release --locked --target "$t"
done
work=target/package/macos
rm -rf "$work"
mkdir -p "$work/bin" "$work/image" dist
for name in fluxdm fluxdm-host; do
    lipo -create -output "$work/bin/$name" \
        "target/aarch64-apple-darwin/release/$name" "target/x86_64-apple-darwin/release/$name"
done
sh scripts/bundle-macos.sh "$work/bin" "$work/image/FluxDM.app"
cp packaging/README-macos.txt "$work/image/Read me first.txt"
ln -s /Applications "$work/image/Applications"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out="dist/FluxDM-$version-macos-universal.dmg"
hdiutil create -quiet -ov -fs HFS+ -format UDZO -volname FluxDM -srcfolder "$work/image" "$out"
echo "$out"
