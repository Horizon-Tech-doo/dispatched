#!/usr/bin/env bash
# Start/stop the Service Bus Emulator used by Dispatched's integration tests.
set -euo pipefail
cd "$(dirname "$0")"

case "${1:-}" in
  up)
    [ -f .env ] || { cp .env.example .env; echo "Created testing/emulator/.env (ACCEPT_EULA=Y accepts the emulator and SQL Server EULAs)."; }
    docker compose --env-file .env up -d
    echo "Waiting for http://localhost:5300/health ..."
    for _ in $(seq 1 90); do
      if curl -fsS http://localhost:5300/health >/dev/null 2>&1; then echo "Emulator healthy."; exit 0; fi
      sleep 2
    done
    echo "Emulator did not become healthy within 180s:"; docker compose logs --tail 80 emulator; exit 1
    ;;
  down)
    docker compose --env-file .env down -v
    ;;
  status)
    curl -fsS http://localhost:5300/health && echo
    ;;
  *)
    echo "usage: $0 up|down|status"; exit 2
    ;;
esac
