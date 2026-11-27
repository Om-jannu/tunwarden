#!/bin/sh
# Inside rust:1.100 (Debian): installs the webview, tray and Xvfb packages,
# builds the CLI, fake-tunnel and the app (debug, no bundle), lays the CLI
# next to the app like a bundle does, and runs scripts/smoke.sh under xvfb.
set -e
apt-get update -qq >/dev/null
apt-get install -y -qq libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev xvfb xauth dbus-x11 >/dev/null 2>&1
mkdir /work && tar -C /src --exclude=./target --exclude=./spikes/target --exclude=./node_modules --exclude=./apps/desktop/node_modules -cf - . | tar -C /work -xf -
cd /work
# The app embeds the built frontend; copy it from the host checkout.
cp -r /src/apps/desktop/dist apps/desktop/dist
cargo build -q -p tunwarden-cli -p fake-tunnel -p tunwarden-desktop
T=/tmp/t/debug
mkdir -p /tmp/app && cp $T/tunwarden-desktop $T/tunwarden /tmp/app/
export XDG_RUNTIME_DIR=/tmp/xdg; mkdir -p -m 700 $XDG_RUNTIME_DIR
xvfb-run -a dbus-launch --exit-with-session sh apps/desktop/scripts/smoke.sh /tmp/app/tunwarden-desktop $T/fake-tunnel
