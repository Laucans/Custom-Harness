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
#
# The same packages, sorted, are cargo machete's ignore list
# ([package.metadata.cargo-machete]): the registry uses each unit only in
# the glue its build script writes into OUT_DIR, which machete does not
# read, so it would flag every one.
#
# One engine: the same line goes into the root Cargo.toml's
# [workspace.dependencies], and a unit that needs the engine (an
# aggregate's Judge, a DataCapability's Prepare) takes it from there:
# `harness-dataguard = { workspace = true }`. Cargo then builds a single
# harness-dataguard, whose Plugin type is the server's; a unit naming its
# own source fails --check. The root line is written when it is missing or
# differs from the server's, so it changes only with the tag.
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

# The engine's line under [workspace.dependencies] of $1, if any.
workspace_engine() {
  awk '
    /^[[:space:]]*\[/ { in_ws = ($0 ~ /^[[:space:]]*\[workspace\.dependencies\][[:space:]]*$/); next }
    in_ws && /^[[:space:]]*harness-dataguard[[:space:]]*=/ { print; exit }
  ' "$1"
}

# True when the Cargo.toml $1 takes harness-dataguard from a source of its
# own instead of the workspace: an inline line or a table without
# `workspace = true`, or a dotted key naming a source.
engine_elsewhere() {
  awk '
    /^[[:space:]]*\[/ {
      if (table && !inherited) bad = 1
      table = ($0 ~ /dependencies\.harness-dataguard\][[:space:]]*$/)
      inherited = 0
      next
    }
    table && /^[[:space:]]*workspace[[:space:]]*=[[:space:]]*true/ { inherited = 1 }
    /^[[:space:]]*harness-dataguard[[:space:]]*=/ && !/workspace[[:space:]]*=[[:space:]]*true/ { bad = 1 }
    /^[[:space:]]*harness-dataguard\.(git|path|version|tag|branch|rev)[[:space:]]*=/ { bad = 1 }
    END { if (table && !inherited) bad = 1; exit !bad }
  ' "$1"
}

failed=0
lines=""
packages=""
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
  if engine_elsewhere "$toml"; then
    if [ "$check" = 1 ]; then
      echo "::error file=$toml::takes harness-dataguard from its own source — write harness-dataguard = { workspace = true }: the registry plants one engine, and another engine's Plugin is not its type"
      failed=1
    else
      echo "::warning file=$toml::takes harness-dataguard from its own source — write harness-dataguard = { workspace = true }"
    fi
  fi
  name=$(package_of "$toml")
  if [ -z "$name" ]; then
    echo "::error file=$toml::no [package] name — a unit is a crate with a name"
    failed=1
    continue
  fi
  lines="$lines$name = { path = \"../../${dir#crates/}\" }
"
  packages="$packages$name
"
done
[ "$failed" = 0 ] || exit 1

# cargo machete's ignore list: one quoted package per line, sorted.
ignored=$(printf '%s' "$packages" | LC_ALL=C sort | sed 's/.*/    "&",/')
if [ -n "$ignored" ]; then
  ignored="ignored = [
$ignored
]"
else
  ignored="ignored = []"
fi

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

# Every unit is used by the glue build.rs writes into OUT_DIR, which cargo
# machete does not read.
[package.metadata.cargo-machete]
$ignored

[dependencies]
$engine"
units=$(printf '%s' "$lines" | LC_ALL=C sort)
[ -z "$units" ] || text="$text
$units"

root=Cargo.toml

if [ "$check" = 1 ]; then
  if [ ! -f "$registry" ] || [ "$(cat "$registry")" != "$text" ]; then
    echo "::error file=$registry::not what the unit folders say — run bash scripts/register.sh and commit it"
    exit 1
  fi
  if [ -f "$root" ] && [ "$(workspace_engine "$root")" != "$engine" ]; then
    echo "::error file=$root::[workspace.dependencies] does not take harness-dataguard as $server does — run bash scripts/register.sh and commit it"
    exit 1
  fi
  exit 0
fi

mkdir -p "$(dirname "$registry")"
printf '%s\n' "$text" > "$registry"
echo "register: $(printf '%s' "$units" | grep -c . || true) unit(s) in $registry"

# The workspace's engine line: replaced where it differs, else added under
# [workspace.dependencies], else in a new section at the end.
if [ -f "$root" ]; then
  current=$(workspace_engine "$root")
  if [ "$current" != "$engine" ]; then
    written=$(ENGINE="$engine" HAS="${current:+1}" awk '
      BEGIN { line = ENVIRON["ENGINE"]; has = (ENVIRON["HAS"] == "1") }
      /^[[:space:]]*\[/ {
        in_ws = ($0 ~ /^[[:space:]]*\[workspace\.dependencies\][[:space:]]*$/)
        print
        if (in_ws && !has && !done) { print line; done = 1 }
        next
      }
      in_ws && /^[[:space:]]*harness-dataguard[[:space:]]*=/ { if (!done) print line; done = 1; next }
      { print }
      END { if (!done) { print ""; print "[workspace.dependencies]"; print line } }
    ' "$root")
    printf '%s\n' "$written" > "$root"
    echo "register: $root takes the engine as $server does"
  fi
fi
