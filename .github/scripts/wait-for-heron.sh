#!/bin/bash
# Polls /health until heron serves it. A bound port is not proof of serving.
for attempt in $(seq 1 50); do
  if curl -fsS --max-time 2 http://127.0.0.1:3100/health > /dev/null 2>&1; then
    exit 0
  fi
  sleep 0.2
done
exit 1
