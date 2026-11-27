#!/bin/sh
# Runs inside rust:1.100: the full test suite, then checks a unit written by
# the real CLI with systemd-analyze. A stand-in systemctl lets `enable` succeed.
set -e
mkdir /work && tar -C /src --exclude=./target --exclude=./spikes/target -cf - . | tar -C /work -xf -
cd /work
cargo test -q --workspace 2>&1 | grep -E "test result|FAILED|panicked" | awk '{print} /test result/ {p+=$4; f+=$6} END {print "TOTAL passed", p, "failed", f}'
cargo build -q -p tunwarden-cli
apt-get update -qq >/dev/null && apt-get install -y -qq systemd >/dev/null 2>&1
mkdir -p /tmp/fakebin /tmp/h
printf '#!/bin/sh\necho "systemctl $*" >> /tmp/systemctl.log\nexit 0\n' > /tmp/fakebin/systemctl
chmod +x /tmp/fakebin/systemctl
HOME=/tmp/h PATH=/tmp/fakebin:$PATH /tmp/t/debug/tunwarden --config /tmp/h/config.toml autostart enable
cat /tmp/systemctl.log
UNIT=/tmp/h/.config/systemd/user/tunwarden.service
cat "$UNIT"
systemd-analyze verify --man=no "$UNIT" && echo "VERIFY OK"
HOME=/tmp/h PATH=/tmp/fakebin:$PATH /tmp/t/debug/tunwarden autostart status
HOME=/tmp/h PATH=/tmp/fakebin:$PATH /tmp/t/debug/tunwarden autostart disable
test ! -f "$UNIT" && echo "REMOVED OK"
