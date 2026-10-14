#!/usr/bin/env bash
# The rules' runtime: how the rules' tests (tests/rules/, contract P) reach
# the product — by identifier, over the shell's wire protocol, as a Micro-UI
# does. Installed by `harness init-repo`, complete: it starts the product
# where the layout puts it, and serves nothing while it is not built, so
# every rule's test stays pending until then. Neither a development nor the
# rules' task changes it (the gate `rule-tests-untouched`): a product that
# cannot be started this way contests it, like a rule's test.
#
# What the product provides, by the layout:
#
#   crates/server/        the one binary serving the shell's wire protocol
#                         (POST /capabilities/<id>, POST /commands,
#                         GET /commands/<id>) and GET /health. It reads
#                         PORT, DATABASE_URL and, when set, FIXTURES_DIR:
#                         the mock-up's fixtures (mock/fixtures/*.json),
#                         loaded into its store as it starts.
#   apps/shell/           the frontend's host: package.json with the scripts
#                         `build` and `serve`; `serve` listens on PORT, reaches
#                         the server at SERVER_URL, and shows a frozen screen
#                         at /screens/<screen>.
#   migrations/*.sql      applied in path order, as scripts/dump-schema.sh does.
#
#   bash scripts/rules-runtime.sh serve
#       Applies the migrations to a scratch database of its own, starts the
#       server on it with the fixtures, then the shell when there is one —
#       in the background, their output in target/rules-runtime/ — and
#       prints what the tests read, one KEY=value a line (the CI appends them
#       to $GITHUB_ENV):
#         RULES_SERVER_URL=http://127.0.0.1:<port>
#         RULES_APP_URL=http://127.0.0.1:<port>
#         RULES_SCREEN_ROUTE=/screens/<screen>
#       Prints nothing while crates/server is not built, or no DATABASE_URL
#       names a Postgres: the tests stay pending. A product that is built
#       and does not start fails: that is a red, not a pending.
#       Never the mock server (mock/src/server.js): it answers the same
#       protocol from the fixtures, and every rule would hold without the
#       product.
#
#   bash scripts/rules-runtime.sh stop
#       Stops what serve started, and drops its scratch database.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
state="$root/target/rules-runtime"
SERVER_PORT="${RULES_SERVER_PORT:-8790}"
APP_PORT="${RULES_APP_PORT:-8791}"
WAIT_SECONDS="${RULES_WAIT_SECONDS:-120}"

say() { echo "rules-runtime: $*" >&2; }

# Answers once `url` does, within WAIT_SECONDS; fails otherwise.
wait_for() {
  local url="$1" what="$2" waited=0
  until curl -fsS -o /dev/null "$url" 2>/dev/null; do
    if [ "$waited" -ge "$WAIT_SECONDS" ]; then
      say "$what does not answer at $url after ${WAIT_SECONDS}s — see $state/"
      return 1
    fi
    sleep 1
    waited=$((waited + 1))
  done
}

# The scratch database the migrations build, its URL on stdout.
scratch_database() {
  local url="$DATABASE_URL" query="" server name
  case "$url" in
    *\?*) query="?${url#*\?}"; url="${url%%\?*}" ;;
  esac
  server="${url%/*}"
  name="rules_runtime_$(date +%s)_$$"
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -qAt -c "CREATE DATABASE $name" >/dev/null
  echo "$name" > "$state/database"
  echo "$DATABASE_URL" > "$state/admin-url"
  local target="$server/$name$query"
  local file
  while IFS= read -r file; do
    if ! psql "$target" -v ON_ERROR_STOP=1 -qAt -f "$root/$file" >/dev/null 2>"$state/migration.err"; then
      say "migration $file failed: $(tail -n 1 "$state/migration.err")"
      return 1
    fi
  done < <(cd "$root" && git ls-files -- '*migrations/*.sql' | LC_ALL=C sort)
  echo "$target"
}

serve() {
  if [ ! -f "$root/crates/server/Cargo.toml" ]; then
    say "no crates/server yet: nothing to serve, the rules' tests stay pending"
    return 0
  fi
  if [ -z "${DATABASE_URL:-}" ]; then
    say "no DATABASE_URL: no store to serve on, the rules' tests stay pending"
    return 0
  fi
  mkdir -p "$state"
  local database
  database="$(scratch_database)"

  (cd "$root" && cargo build --quiet --manifest-path crates/server/Cargo.toml)
  local binary
  binary="$(cd "$root" && cargo metadata --format-version 1 --no-deps --manifest-path crates/server/Cargo.toml \
    | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const m=JSON.parse(s);const p=m.packages.find(p=>p.manifest_path.endsWith("crates/server/Cargo.toml"));const b=p&&p.targets.find(t=>t.kind.includes("bin"));process.stdout.write(`${m.target_directory}/debug/${b?b.name:""}`)})')"
  if [ ! -x "$binary" ]; then
    say "crates/server builds no binary ($binary)"
    return 1
  fi
  local fixtures=""
  [ -d "$root/mock/fixtures" ] && fixtures="$root/mock/fixtures"
  PORT="$SERVER_PORT" DATABASE_URL="$database" FIXTURES_DIR="$fixtures" \
    nohup "$binary" >"$state/server.log" 2>&1 &
  echo $! > "$state/server.pid"
  local server_url="http://127.0.0.1:$SERVER_PORT"
  wait_for "$server_url/health" "the server"
  echo "RULES_SERVER_URL=$server_url"

  if [ -f "$root/apps/shell/package.json" ]; then
    (cd "$root/apps/shell" && npm install --no-audit --no-fund >"$state/shell-install.log" 2>&1 && npm run build >"$state/shell-build.log" 2>&1)
    (cd "$root/apps/shell" && PORT="$APP_PORT" SERVER_URL="$server_url" nohup npm run serve >"$state/shell.log" 2>&1 & echo $! > "$state/shell.pid")
    local app_url="http://127.0.0.1:$APP_PORT"
    wait_for "$app_url/" "the shell"
    echo "RULES_APP_URL=$app_url"
    echo "RULES_SCREEN_ROUTE=/screens/<screen>"
  fi
}

stop() {
  local pid
  for name in shell server; do
    if [ -f "$state/$name.pid" ]; then
      pid="$(cat "$state/$name.pid")"
      # npm runs the shell as its child: both go.
      pkill -P "$pid" 2>/dev/null || true
      kill "$pid" 2>/dev/null || true
      rm -f "$state/$name.pid"
    fi
  done
  if [ -f "$state/database" ] && [ -f "$state/admin-url" ]; then
    psql "$(cat "$state/admin-url")" -qAt -c "DROP DATABASE IF EXISTS $(cat "$state/database")" >/dev/null 2>&1 || true
    rm -f "$state/database" "$state/admin-url"
  fi
}

case "${1:-}" in
  serve) serve ;;
  stop) stop ;;
  *)
    echo "usage: bash scripts/rules-runtime.sh serve | stop" >&2
    exit 2
    ;;
esac
