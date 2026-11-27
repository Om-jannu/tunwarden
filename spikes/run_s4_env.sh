#!/bin/sh
# Spike S4b: sweep by inherited environment tag. Usage: run_s4_env.sh BIN [--pdeathsig|--leader-exits]
set -u
BIN=$1; EXTRA=${2:-}; STATE=$(mktemp)
echo "== $(uname -s) $(uname -m) ${EXTRA:-(default)}"
"$BIN" parent "$STATE" $EXTRA & PARENT=$!
sleep 1
read PID PGID ST TAG < "$STATE"
kill -9 "$PARENT"; wait "$PARENT" 2>/dev/null; sleep 1
echo "after kill -9, leftover sleep 300 processes: $(pgrep -f 'sleep 300' | tr '\n' ' ')"
"$BIN" sweep "$STATE"
"$BIN" sweep-env "$STATE"
echo "after sweep-env, leftover sleep 300 processes: $(pgrep -f 'sleep 300' | tr '\n' ' ')"
rm -f "$STATE"
