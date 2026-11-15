#!/usr/bin/env bash
# Starts or stops the local test servers (see docker-compose.yml for details).
# Usage: bash dev/servers.sh up|down
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KEYS="$HERE/keys"
COMPOSE=(docker compose -f "$HERE/docker-compose.yml")

random() { LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c "$1"; }

generate_credentials() {
  mkdir -p "$KEYS"
  chmod 700 "$KEYS"
  if [ ! -f "$KEYS/id_ed25519" ]; then
    ssh-keygen -q -t ed25519 -N '' -C 'portolano-dev-test' -f "$KEYS/id_ed25519"
  fi
  cp "$KEYS/id_ed25519.pub" "$KEYS/authorized_keys"
  if [ ! -f "$KEYS/test.env" ]; then
    umask 077
    printf 'TEST_PASSWORD=%s\nVNC_PASSWORD=%s\n' "$(random 20)" "$(random 8)" > "$KEYS/test.env"
  fi
}

case "${1:-}" in
  up)
    generate_credentials
    "${COMPOSE[@]}" up -d --build
    ;;
  down)
    "${COMPOSE[@]}" down
    ;;
  *)
    echo "usage: bash dev/servers.sh up|down" >&2
    exit 2
    ;;
esac
