#!/usr/bin/env bash
# The product's PostgreSQL, in Docker — see docs/PERSISTENCE.md.
#
#   scripts/postgres.sh up          start it (created and provisioned the first time)
#   scripts/postgres.sh provision   bring the roles back in line with .env.local
#   scripts/postgres.sh status      is it up, healthy, on which port
#   scripts/postgres.sh psql [db]   a psql as the superuser
#   scripts/postgres.sh logs        follow its log
#   scripts/postgres.sh down        stop it; the data stays in the volume harness-pgdata
#   scripts/postgres.sh reset       stop it and DELETE the volume (asks first)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
env_file="$root/.env.local"
[ -f "$env_file" ] || { echo "no .env.local at $root — copy .env.example and fill DATABASE_URL" >&2; exit 1; }
compose=(docker compose --env-file "$env_file" -f "$root/docker/postgres/compose.yml")

wait_healthy() {
  for _ in $(seq 1 60); do
    [ "$(docker inspect -f '{{.State.Health.Status}}' harness-postgres 2>/dev/null)" = healthy ] && return 0
    sleep 1
  done
  echo "harness-postgres is not healthy after a minute: scripts/postgres.sh logs" >&2
  return 1
}

case "${1:-status}" in
  up)
    "${compose[@]}" up -d
    wait_healthy
    echo "harness-postgres is up on 127.0.0.1:${POSTGRES_PORT:-5432}"
    ;;
  provision)
    wait_healthy
    docker exec harness-postgres sh /docker-entrypoint-initdb.d/10-provision.sh
    ;;
  status)
    docker ps --filter name=harness-postgres --format '{{.Names}}  {{.Status}}  {{.Ports}}'
    ;;
  psql)
    docker exec -it harness-postgres psql -U postgres "${2:-postgres}"
    ;;
  logs)
    docker logs -f harness-postgres
    ;;
  down)
    "${compose[@]}" down
    ;;
  reset)
    read -r -p "Delete the volume harness-pgdata and every database in it? [y/N] " answer
    [ "$answer" = y ] || { echo "kept"; exit 0; }
    "${compose[@]}" down -v
    ;;
  *)
    sed -n '2,10p' "$0" >&2
    exit 2
    ;;
esac
