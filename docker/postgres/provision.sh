#!/bin/sh
# Creates, or brings back in line, what the product's runs connect with:
#
#   the application role  (DATABASE_URL)          LOGIN CREATEDB CREATEROLE —
#                           it migrates, and tests create a database each;
#                           a member of pg_signal_backend, so a test's
#                           DROP DATABASE … WITH (FORCE) ends the sessions
#                           another role (the read-only login) left on it
#   its database          (DATABASE_URL's name)   owned by the application role
#   the read-only login   (DATABASE_URL_READONLY) LOGIN, nothing else — it
#                           reads through the role named by
#                           DATABASE_READONLY_ROLE, created NOLOGIN here when
#                           the migrations have not yet (as the target's CI does)
#
# Idempotent: run by the image's entrypoint on a new volume, and again by
# `scripts/postgres.sh provision` whenever `.env.local` changes. Passwords are
# taken from the URLs as written: percent-encoding is not decoded.
set -eu

# postgres://user:password@host:port/database?query → three variables.
parse() {
  rest="${1#*://}"
  creds="${rest%%@*}"
  where="${rest#*@}"
  user="${creds%%:*}"
  pass="${creds#*:}"
  db="${where#*/}"
  db="${db%%\?*}"
}

sql() {
  psql -v ON_ERROR_STOP=1 --username postgres --dbname postgres -qAt "$@"
}

parse "$APP_DATABASE_URL"
app="$user"
sql -v role="$app" -v pass="$pass" -v db="$db" <<'SQL'
SELECT format('CREATE ROLE %I', :'role')
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'role') \gexec
SELECT format('ALTER ROLE %I LOGIN CREATEDB CREATEROLE NOSUPERUSER PASSWORD %L', :'role', :'pass') \gexec
SELECT format('GRANT pg_signal_backend TO %I', :'role')
 WHERE NOT pg_has_role(:'role', 'pg_signal_backend', 'USAGE') \gexec
SELECT format('CREATE DATABASE %I OWNER %I', :'db', :'role')
 WHERE NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = :'db') \gexec
SQL
echo "provisioned the application role $app and its database $db"

if [ -n "${READONLY_DATABASE_URL:-}" ]; then
  parse "$READONLY_DATABASE_URL"
  if [ "$user" != "$app" ]; then
    sql -v role="$user" -v pass="$pass" <<'SQL'
SELECT format('CREATE ROLE %I', :'role')
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'role') \gexec
SELECT format('ALTER ROLE %I LOGIN NOCREATEDB NOCREATEROLE NOSUPERUSER PASSWORD %L', :'role', :'pass') \gexec
SQL
    echo "provisioned the read-only login $user"
    if [ -n "${READONLY_ROLE:-}" ] && [ "$READONLY_ROLE" != "$user" ]; then
      sql -v login="$user" -v role="$READONLY_ROLE" <<'SQL'
SELECT format('CREATE ROLE %I NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT', :'role')
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'role') \gexec
SELECT format('GRANT %I TO %I', :'role', :'login') \gexec
SQL
      echo "the read-only login $user reads as $READONLY_ROLE"
    fi
  fi
fi
