#!/usr/bin/env bash
# The registry of the product's units — installed by `harness init-repo`;
# the repository owns it from here.
#
#   bash scripts/register.sh            # writes crates/dataguard/registry/Cargo.toml
#   bash scripts/register.sh --check    # exits 1 when the committed one differs
#
# A unit is a folder: crates/<system>/aggregates/<Aggregate>/,
# crates/<system>/data-capabilities/<name>/ or
# crates/<system>/capabilities/<name>/. The registry crate
# (crates/dataguard/registry) depends on every one that is a crate, and its
# build script turns them into the plugins the server plants. Its
# Cargo.toml is this script's, never edited by hand: adding a unit is its
# folder plus one run of this script, and two units added in parallel are
# merged by running it again, never by resolving the file.
#
# What it writes: the package header, the engine's dependency line copied
# from crates/dataguard/server/Cargo.toml (the tag is written there once),
# and one line per unit crate, sorted, named after the unit's package. A
# unit whose manifest says "host": "hostless" runs in no server and is left
# out. The registry's build script fails the build, naming the units, when
# a folder and this file disagree.
set -euo pipefail

cd "$(dirname "$0")/.."

check=0
[ "${1:-}" = "--check" ] && check=1

registry=crates/dataguard/registry/Cargo.toml
server=crates/dataguard/server/Cargo.toml

[ -f "$server" ] || { echo "::error file=$server::missing — the registry copies the engine's line from it"; exit 1; }
engine=$(grep -m1 -E '^harness-dataguard[[:space:]]*=' "$server" || true)
[ -n "$engine" ] || { echo "::error file=$server::no harness-dataguard line to copy"; exit 1; }

# The package name a unit's Cargo.toml declares under [package].
package_of() {
  awk '
    /^[[:space:]]*\[/ { in_package = ($0 ~ /^[[:space:]]*\[package\][[:space:]]*$/) ; next }
    in_package && /^[[:space:]]*name[[:space:]]*=/ {
      sub(/^[^=]*=[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit
    }
  ' "$1"
}

failed=0
lines=""
for toml in crates/*/aggregates/*/Cargo.toml crates/*/data-capabilities/*/Cargo.toml crates/*/capabilities/*/Cargo.toml; do
  [ -f "$toml" ] || continue
  dir=$(dirname "$toml")
  hostless=0
  for manifest in "$dir/aggregate.json" "$dir/data-capability.json" "$dir/capability.json"; do
    if [ -f "$manifest" ] && tr -d ' \t\r\n' < "$manifest" | grep -q '"host":"hostless"'; then
      hostless=1
    fi
  done
  [ "$hostless" = 0 ] || continue
  name=$(package_of "$toml")
  if [ -z "$name" ]; then
    echo "::error file=$toml::no [package] name — a unit is a crate with a name"
    failed=1
    continue
  fi
  lines="$lines$name = { path = \"../../${dir#crates/}\" }
"
done
[ "$failed" = 0 ] || exit 1

text="# Written by scripts/register.sh: never edited by hand. A unit folder
# added under crates/<system>/{aggregates,data-capabilities,capabilities}/
# is registered by running the script again; the build fails until it is.
[package]
name = \"dataguard-registry\"
version = \"0.1.0\"
edition = \"2024\"
description = \"Every unit of the product, gathered: what the server plants.\"
publish = false
build = \"build.rs\"

[dependencies]
$engine"
units=$(printf '%s' "$lines" | LC_ALL=C sort)
[ -z "$units" ] || text="$text
$units"

if [ "$check" = 1 ]; then
  if [ ! -f "$registry" ] || [ "$(cat "$registry")" != "$text" ]; then
    echo "::error file=$registry::not what the unit folders say — run bash scripts/register.sh and commit it"
    exit 1
  fi
  exit 0
fi

mkdir -p "$(dirname "$registry")"
printf '%s\n' "$text" > "$registry"
echo "register: $(printf '%s' "$units" | grep -c . || true) unit(s) in $registry"
