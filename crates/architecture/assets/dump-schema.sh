#!/usr/bin/env bash
# Prints the database schema this checkout's migrations build, as
# `pg_dump --schema-only` writes it.
#
# The harness runs it after every merge on main, main_agent and the milestone
# branches, and archives what it prints on the branch `harness/data-model`,
# where the factory view reads every version. Adapt it to the stack; keep the
# contract:
#
#   in   DATABASE_URL — a Postgres role allowed to create a database
#   out  the DDL on stdout, exit 0
#        nothing printed, exit 0 — this checkout has no database yet
#        any other exit — a failure, its last stderr line says why
#
# This default applies every tracked `migrations/*.sql`, sorted by path, to a
# scratch database it drops on the way out.
set -euo pipefail

: "${DATABASE_URL:?DATABASE_URL must name a Postgres role allowed to create a database}"

files=()
while IFS= read -r file; do
  files+=("$file")
done < <(git ls-files -- '*migrations/*.sql' | LC_ALL=C sort)
if [ "${#files[@]}" -eq 0 ]; then
  exit 0
fi

# The URL without its database name, and its query kept aside.
url="$DATABASE_URL"
query=""
case "$url" in
  *\?*) query="?${url#*\?}"; url="${url%%\?*}" ;;
esac
server="${url%/*}"
scratch="schema_dump_$(date +%s)_$$"

psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -qAt -c "CREATE DATABASE $scratch" >/dev/null
cleanup() {
  psql "$DATABASE_URL" -qAt -c "DROP DATABASE IF EXISTS $scratch" >/dev/null 2>&1 || true
}
trap cleanup EXIT

target="$server/$scratch$query"
for file in "${files[@]}"; do
  if ! psql "$target" -v ON_ERROR_STOP=1 -qAt -f "$file" >/dev/null 2>/tmp/dump-schema.$$.err; then
    echo "migration $file failed: $(tail -n 1 /tmp/dump-schema.$$.err)" >&2
    rm -f /tmp/dump-schema.$$.err
    exit 1
  fi
done
rm -f /tmp/dump-schema.$$.err

# `\restrict` lines carry a random key on every dump: dropped, so an unchanged
# schema dumps to the same bytes.
pg_dump --schema-only --no-owner --no-privileges "$target" | grep -v '^\\\(un\)\{0,1\}restrict '
