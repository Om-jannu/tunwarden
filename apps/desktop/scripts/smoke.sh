#!/bin/sh
# Smoke test for a built desktop app: starts it against a temporary config
# and state dir, checks that it started the daemon with the CLI that sits
# next to it, that a tunnel runs through that daemon, that a second launch
# does not start a second app, and that tunnels outlive the app.
#
#   scripts/smoke.sh APP_BINARY FAKE_TUNNEL
#
# APP_BINARY is the app's executable (in a bundle: Tunwarden.app/Contents/MacOS/
# tunwarden-desktop). On Linux CI run it under xvfb-run.
set -u
app=$1
fake=$2
dir=$(mktemp -d)
export TUNWARDEN_CONFIG="$dir/config.toml" TUNWARDEN_STATE_DIR="$dir/state"
cli="$(dirname "$app")/tunwarden"
fail() { echo "FAIL: $*"; cat "$dir/app.log" 2>/dev/null | tail -20; "$cli" daemon stop </dev/null >/dev/null 2>&1; exit 1; }
[ -x "$cli" ] || fail "no tunwarden CLI next to $app"
# The app allows one copy per user, so a running copy would take over the test.
# `comm` is the program itself, so this script's own arguments never match.
if ps -eo comm= | grep -q "tunwarden-desk"; then
  echo "SKIP: another copy of the app is running; quit it and run this again"
  exit 2
fi
port=17655
cat > "$TUNWARDEN_CONFIG" <<EOF
version = 1
[[tunnels]]
name = "demo"
command = "'$fake' --listen 127.0.0.1:$port"
forwards = [{ local_port = $port }]
EOF

"$app" --background > "$dir/app.log" 2>&1 &
pid=$!
i=0
until grep -q "listening on" "$dir/state/daemon.log" 2>/dev/null; do
  i=$((i + 1)); [ $i -gt 60 ] && fail "the app did not start a daemon"; sleep 0.5
done
kill -0 $pid 2>/dev/null || fail "the app exited"
echo "ok: the app started a daemon"
daemon=$(ps -eo command | grep "daemon run" | grep "$dir/config.toml" | grep -v grep | head -1)
case "$daemon" in
  "$cli "*) echo "ok: the daemon runs the bundled CLI" ;;
  *) fail "the daemon is not the bundled CLI: $daemon" ;;
esac

"$app" > "$dir/second.log" 2>&1 &
second=$!
sleep 3
if kill -0 $second 2>/dev/null; then kill $second; fail "a second launch kept running"; fi
echo "ok: a second launch hands over to the running app"

"$cli" start demo --wait 20 </dev/null | grep -q running || fail "the tunnel did not start"
echo "ok: a tunnel runs through the app's daemon"
kill $pid; sleep 1
"$cli" status demo </dev/null | grep -q running || fail "the tunnel stopped with the app"
echo "ok: the tunnel keeps running after the app quits"
"$cli" daemon stop </dev/null >/dev/null
rm -rf "$dir"
echo "PASS"
