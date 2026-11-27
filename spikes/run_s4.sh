#!/bin/sh
# Spike S4 driver. Usage: run_s4.sh PATH_TO_s4_sweep [--pdeathsig]
set -u
BIN=$1; EXTRA=${2:-}
STATE=$(mktemp)

survivors() { pgrep -g "$1" | tr '\n' ' '; }

echo "== $(uname -s) $(uname -m) ${EXTRA:-(no pdeathsig)}"
"$BIN" parent "$STATE" $EXTRA &
PARENT=$!
sleep 1
read PID PGID ST < "$STATE"
echo "before kill -9: group $PGID members: $(survivors "$PGID")"
kill -9 "$PARENT"; wait "$PARENT" 2>/dev/null
sleep 1
echo "after  kill -9: group $PGID members: $(survivors "$PGID")"

echo "-- sweep with a wrong start time (PID reuse guard)"
"$BIN" sweep "$STATE" --wrong-start
echo "   group $PGID members: $(survivors "$PGID")"

echo "-- real sweep"
"$BIN" sweep "$STATE"
echo "   group $PGID members: $(survivors "$PGID")"
pkill -g "$PGID" 2>/dev/null
rm -f "$STATE"
