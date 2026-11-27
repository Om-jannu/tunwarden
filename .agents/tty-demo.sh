#!/bin/sh
# Runs the CLI in a pseudo-terminal (script) so the terminal-only output shows.
set -u
B=/Users/omjannu/Projects/tunwarden/target/debug/tunwarden
F=/Users/omjannu/Projects/tunwarden/target/debug/fake-tunnel
D=/tmp/tw-tty
rm -rf "$D"; mkdir -p "$D"
export TUNWARDEN_STATE_DIR="$D/state" TUNWARDEN_CONFIG="$D/config.toml" COLUMNS=100
cat > "$TUNWARDEN_CONFIG" <<EOF
version = 1
[[tunnels]]
name = "staging-db"
command = "'$F' --listen 127.0.0.1:17433 --listen-after 1s"
groups = ["daily"]
forwards = [{ local_port = 17433 }]
[[tunnels]]
name = "broken"
command = "'$F' --exit-after 200ms --exit-code 1"
max_retries = 1
groups = ["daily"]
forwards = [{ local_port = 17434 }]
EOF
tty() { script -q /dev/null "$@" </dev/null; }
echo "### start --group daily (terminal)"; tty "$B" start --group daily --wait 15
echo "### status (terminal)"; tty "$B" status
echo "### logs -n 3 (terminal: local time)"; tty "$B" logs staging-db -n 3
echo "### logs -n 1 (piped: stored line)"; "$B" logs staging-db -n 1 </dev/null
echo "### bad config (terminal)"
cp "$TUNWARDEN_CONFIG" "$D/good.toml"
printf 'version = 1\n[[tunnels]]\nname = "db"\ntype = "ssh-local"\nhost = "-oProxyCommand=x"\nforwards = [{ local_port = 5433, remote_port = 5432 }]\n' > "$TUNWARDEN_CONFIG"
tty "$B" list
cp "$D/good.toml" "$TUNWARDEN_CONFIG"
"$B" daemon stop </dev/null >/dev/null
