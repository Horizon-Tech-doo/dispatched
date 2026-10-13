#!/usr/bin/env bash
# Start/stop the Service Bus Emulator used by Dispatched's integration tests.
set -euo pipefail
cd "$(dirname "$0")"

# .env is needed by every subcommand that touches compose (the compose file
# requires its variables), so create it once up front.
[ -f .env ] || { cp .env.example .env; echo "Created testing/emulator/.env (ACCEPT_EULA=Y accepts the emulator and SQL Server EULAs)."; }

# Compose args, built once. On arm64 hosts add the override that swaps the
# amd64-only SQL Server image for Azure SQL Edge (opt out with
# DISPATCHED_EMULATOR_COMPOSE_ARM64=0).
COMPOSE=(docker compose --env-file .env -f docker-compose.yml)
case "$(uname -m)" in
  arm64|aarch64)
    if [ "${DISPATCHED_EMULATOR_COMPOSE_ARM64:-1}" != "0" ]; then
      COMPOSE+=(-f docker-compose.arm64.yml)
    fi
    ;;
esac

case "${1:-}" in
  up)
    "${COMPOSE[@]}" up -d
    echo "Waiting for http://localhost:5300/health ..."
    for _ in $(seq 1 90); do
      if curl -fsS --max-time 5 http://localhost:5300/health >/dev/null 2>&1; then echo "Emulator healthy."; exit 0; fi
      sleep 2
    done
    echo "Emulator did not become healthy within 180s:"; "${COMPOSE[@]}" logs --tail 80 emulator; exit 1
    ;;
  down)
    "${COMPOSE[@]}" down -v
    ;;
  logs)
    "${COMPOSE[@]}" logs --tail 200 emulator
    ;;
  status)
    curl -fsS --max-time 5 http://localhost:5300/health && echo
    ;;
  *)
    echo "usage: $0 up|down|logs|status"; exit 2
    ;;
esac
