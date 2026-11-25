#!/bin/sh
# Build FluxDM, install it as an app bundle and register the browser helper.
# The bundle lives outside Desktop, Documents and Downloads on purpose: macOS
# privacy protection stops a browser from launching a helper stored there.
set -eu
cd "$(dirname "$0")/.."
[ "${SKIP_BUILD:-}" = 1 ] || cargo build --release --locked

app="${FLUXDM_APP_DIR:-$HOME/Applications}/FluxDM.app"
sh scripts/bundle-macos.sh target/release "$app"
# Make Finder pick up a changed icon.
touch "$app"

"$app/Contents/MacOS/fluxdm-host" install
cat <<MSG

Installed: $app
Browser extension: open chrome://extensions, turn on Developer mode,
choose "Load unpacked" and select:
  $app/Contents/Resources/extension
(In the file picker press Cmd+Shift+G and paste that path.)
MSG
