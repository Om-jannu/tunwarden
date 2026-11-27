#!/bin/sh
# Runs the user flow against fake tunnels and a fake login tool, printing real output.
set -u
ROOT=/Users/omjannu/Projects/tunwarden
B=$ROOT/target/debug/tw-test-cli
FAKE=$ROOT/target/debug/fake-tunnel
D=/tmp/tw-demo
rm -rf "$D"; mkdir -p "$D/bin"
export TUNWARDEN_STATE_DIR="$D/state" TUNWARDEN_CONFIG="$D/config.toml"
cat > "$D/bin/mycloud" <<'EOF'
#!/bin/sh
dir=$(dirname "$0")
case "$1" in
  status) [ -f "$dir/expired" ] && { echo "ERROR: Reauthentication failed. cannot prompt" >&2; exit 1; }; exit 0 ;;
  login) rm -f "$dir/expired"; echo "Opening browser... You are now logged in as [$MYCLOUD_ACCOUNT]."; exit 0 ;;
esac
EOF
chmod +x "$D/bin/mycloud"
run() { echo "\$ tunwarden $*"; "$B" "$@" </dev/null 2>&1 | sed "s#$D#~/demo#g; s#$FAKE#fake-tunnel#g"; echo "(exit $?)"; echo; }

run init
cat > "$TUNWARDEN_CONFIG" <<EOF
version = 1
[tools]
path = ["$D/bin"]

[[logins]]
name = "work"
check = "mycloud status"
login = "mycloud login"
expired = ["Reauthentication failed"]
env = { MYCLOUD_ACCOUNT = "me@example.com" }

[[profiles]]
name = "staging"
login = "work"
env = { MYCLOUD_PROJECT = "staging-123" }

[[profiles]]
name = "prod"
login = "work"
production = true
env = { MYCLOUD_PROJECT = "prod-456" }

[[tunnels]]
name = "staging-db"
profile = "staging"
command = "'$FAKE' --listen 127.0.0.1:15433"
groups = ["daily"]
forwards = [{ local_port = 15433 }]

[[tunnels]]
name = "prod-api"
profile = "prod"
command = "'$FAKE' --listen 127.0.0.1:18080"
forwards = [{ local_port = 18080 }]
EOF
run list
run start --group daily
run start prod-api
run start prod-api --yes
run status
run auth
touch "$D/bin/expired"
sleep 1
run auth staging
run status
run login staging
sleep 2
run status
run logs staging-db -n 6
printf '%s\n' "alias cache='ssh -f -N -L 6380:redis.internal:6379 bastion'" "alias kafka='MYCLOUD_PROJECT=staging-123 mycloud forward kafka-1 --local-port=19092'" > "$D/aliases"
run import "$D/aliases"
run add metrics --command "'$FAKE' --listen 127.0.0.1:19090" --forward 19090 --group daily
run doctor
run stop --all
run daemon stop
