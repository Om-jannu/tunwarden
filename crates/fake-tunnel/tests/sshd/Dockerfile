# OpenSSH server for the SSH integration tests (`tests/ssh.rs`, PRD AC-11).
# Each container runs sshd for user `tw` and an echo service on
# 127.0.0.1:7000 that only tunnels can reach.
FROM alpine:3.24
RUN apk add --no-cache openssh-server socat \
 && adduser -D -s /bin/sh tw \
 && sed -i 's/^tw:!/tw:*/' /etc/shadow \
 && ssh-keygen -A
COPY sshd_config /etc/ssh/sshd_config
COPY entrypoint.sh /entrypoint.sh
RUN chmod 755 /entrypoint.sh
EXPOSE 22
ENTRYPOINT ["/entrypoint.sh"]
