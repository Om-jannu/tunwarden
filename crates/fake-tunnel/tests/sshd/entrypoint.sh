#!/bin/sh
# AUTHORIZED_KEY is the public key the test allows.
set -e
mkdir -p /home/tw/.ssh
printf '%s\n' "$AUTHORIZED_KEY" > /home/tw/.ssh/authorized_keys
chown -R tw:tw /home/tw/.ssh
chmod 700 /home/tw/.ssh
chmod 600 /home/tw/.ssh/authorized_keys
socat TCP-LISTEN:7000,bind=127.0.0.1,fork,reuseaddr EXEC:cat &
exec /usr/sbin/sshd -D -e
