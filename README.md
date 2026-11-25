# FluxDM

A free, open download manager written in Rust, with a Chrome extension. No
accounts, trial, licence keys, adverts or telemetry.

## Features

- Up to 32 connections per download with dynamic segmentation
- Pause, resume and recovery after a crash or restart
- Refuses to mix two versions of a file that changed on the server
- Retries, and backs off when a server limits connections
- Global and per-download speed limits
- Bulk download window: grab every link on a web page, find links in pasted text
  or an imported file, tick what you want
- Copying a file address offers to download it (switch off in Options; macOS may
  ask for permission to read the clipboard; images and web pages are ignored)
- Queue with a concurrency limit and a daily or weekly schedule
- Categories, search, batch patterns such as `file[01-20].zip`
- HTTP, HTTPS, FTP, HTTP and SOCKS5 proxies, Basic and Digest authentication
- HLS video: best quality, AES-128, separate audio merged when `ffmpeg` is present
- SHA-256, SHA-1 and MD5 verification
- Replace an expired link and continue
- Browser extension: take over downloads (by file extension or server file type,
  or every download), download one link, pick from all or selected links in a
  window with type filters, and download media detected on a page. Alt+click skips
  capture
- Command line downloader

## Not included

Multi-level site crawler (page grabbing reads one page and does not run its scripts), DASH, RTMP, FTPS, live streams, DRM-protected media, YouTube,
Firefox and Safari extensions, capture outside the browser, downloads started by a
form POST (they stay in the browser), several named queues, shutdown when the
queue ends, tray icon, start at login, translations. FTP uses one connection.

## Speed

Extra connections help when the server limits each connection. They cannot exceed
your internet line or a per-user server limit. Measured against a local server
limiting each connection to 1 MiB/s (32 MiB file): curl 1.0 MiB/s; FluxDM with 8
connections 7.4, 16 (default) 13.8, 32 24.7 MiB/s.

## Builds for every desktop platform

`.github/workflows/build.yml` runs the tests on Linux, macOS and Windows, then
builds these files (download them from the workflow run's Artifacts):

| Platform | Output |
|---|---|
| macOS (Apple silicon and Intel) | universal `.dmg` |
| Windows x64 and ARM64 | portable `.zip` |
| Linux x64 and ARM64 | `.deb` and `.tar.gz` |
| Chromium browsers | extension `.zip` |

To publish a version, set `version` in `Cargo.toml`, commit, then push a tag such
as `v0.1.0`. The workflow creates a draft release with `SHA256SUMS.txt`; review
and publish it on GitHub. The apps are not code signed or notarised, so macOS and
Windows warn on first launch. There are no Android or iOS builds.

Local packaging: `scripts/package-macos.sh` (needs both Apple Rust targets),
`scripts/package-linux.sh`, `scripts/package-windows.ps1`.

## Look and icons

The dark interface follows the zinc palette of shadcn/ui. It uses the Geist fonts
and Lucide icons; their licences are in `crates/app/assets/`. The app icon source
is `assets/icon.svg`, and `scripts/make-icons.mjs` regenerates every size.

## Install on macOS

    ./scripts/install-macos.sh

This installs `~/Applications/FluxDM.app` and registers the browser helper. Then
open `chrome://extensions`, enable Developer mode, choose **Load unpacked** and
select the folder the script prints.

Do not register the helper from a build inside Desktop, Documents or Downloads:
macOS privacy protection stops the browser from launching it, and the extension
waits without an error.

## Windows

Build on Windows with `cargo build --release`, or cross-compile with MinGW-w64:

    rustup target add x86_64-pc-windows-gnu
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
      cargo build --release --target x86_64-pc-windows-gnu

Keep `fluxdm.exe` and `fluxdm-host.exe` in the same permanent folder, run
`fluxdm-host.exe install`, then load the `extension` folder in Chrome or Edge.
The Windows build compiles but has not yet been run on Windows.

## Linux

`cargo build --release`, then `target/release/fluxdm-host install`. Not yet tested.

## Command line

    fluxdm-host get URL -o FOLDER -n 16 -l 2048 -c sha256:HEX
    fluxdm-host add URL...

## Tests

    cargo test --workspace
    node --test extension/tests/rules.test.mjs

The engine tests use a local server that drops connections, limits them, ignores
ranges and changes files, and compare results byte for byte.

## Layout

    crates/core   engine: segments, HTTP, HLS, FTP, queue, local IPC
    crates/app    desktop window (egui)
    crates/host   browser helper, installer, command line
    extension     Chrome and Edge extension (Manifest V3)

## Privacy

Cookies and passwords needed by unfinished downloads are stored, unencrypted, in
files only your user account can read. They are removed from a download's record
when it completes. On Windows the files rely on the profile folder's permissions.
Private-window downloads are not captured.

Download only content you have the right to download.
