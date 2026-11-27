#!/bin/sh
# Starts the real app against a temporary config and state dir, checks that it
# started its own daemon through the CLI next to it, then cleans up.
set -u
T=/Users/omjannu/Projects/tunwarden/target/debug
D=/tmp/tw-app
rm -rf "$D"; mkdir -p "$D"
export TUNWARDEN_CONFIG="$D/config.toml" TUNWARDEN_STATE_DIR="$D/state"
cat > "$TUNWARDEN_CONFIG" <<EOF
version = 1
[[tunnels]]
name = "demo"
command = "'$T/fake-tunnel' --listen 127.0.0.1:17555"
forwards = [{ local_port = 17555 }]
EOF
"$T/tunwarden-desktop" > "$D/app.log" 2>&1 &
APP=$!
sleep 6
echo "app alive: $(kill -0 $APP 2>/dev/null && echo yes || echo no)"
echo "daemon started by the app:"
grep -E "listening on" "$D/state/daemon.log" | tail -1
pgrep -fl "tunwarden --config $D/config.toml daemon run" | head -1
echo "start through the daemon the app started:"
"$T/tunwarden" start demo </dev/null | tail -2
sleep 2
echo "app log:"; grep -viE "^$" "$D/app.log" | tail -5
kill $APP; sleep 1
echo "app gone: $(kill -0 $APP 2>/dev/null && echo no || echo yes)"
"$T/tunwarden" status </dev/null | tail -2
"$T/tunwarden" daemon stop </dev/null
echo "leftover fake-tunnel: $(pgrep -x fake-tunnel | wc -l)"
