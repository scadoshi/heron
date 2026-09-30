#!/bin/bash
# Polls steller with a RESP PING until it answers PONG. Speaks the protocol over a
# socket, so there is no client to install.
ping() {
  exec 3<>/dev/tcp/127.0.0.1/3000 || return 1
  printf '*1\r\n$4\r\nPING\r\n' >&3
  head -c 5 <&3
  exec 3>&-
}
for attempt in $(seq 1 50); do
  if [ "$(ping 2>/dev/null)" = "+PONG" ]; then
    exit 0
  fi
  sleep 0.2
done
exit 1
