#!/usr/bin/env bash
# The isolation of the slices, checked on the real dependency graph —
# installed by `harness init-repo`; the repository owns it from here.
#
#   bash scripts/isolation.sh
#
# What the architecture asks (docs/ARCHITECTURE.md, principle 5, rules 2
# and 3): the world of reading is parallelisable because no slice knows
# another. So, on the graph `cargo metadata` answers — not on the text of a
# Cargo.toml, which misses a workspace-level dependency and a transitive one:
#
# 1. A Capability (crates/<system>/capabilities/<name>/) or a DataCapability
#    (crates/<system>/data-capabilities/<name>/) depends on no other
#    Capability or DataCapability, directly or transitively.
# 2. It depends only on crates of its own system (crates/<system>/…),
#    never on another system's. A DataCapability's system is its id's
#    namespace, the folder it sits under (campaign.saveEntity under
#    crates/campaign/): the DataGuard owns it, it does not house it.
#    A port (crates/ports/<port>/, contract Q) belongs to no system: every
#    system reaches the outside through it.
# 3. A crate shared inside a system — one that a Capability or a
#    DataCapability depends on — depends on no Capability, no DataCapability
#    and no crate of another system: plumbing, never a bridge.
#
# 4. A Capability reads through its system's read crate (`<system>-read`,
#    a read-only connection with no write API), never through a database
#    driver of its own: `sqlx`, `tokio-postgres`, `postgres`, `diesel`,
#    `sea-orm`, `rusqlite`, `mysql` are not among its dependencies. The
#    read-only rule is then a fact of the types, and the read-only role is
#    defence in depth.
#
# A dev-dependency counts for none of this: a test-only crate (the
# conformance runner under crates/platform/) is not a bridge at run time.
#
# And code is shared through a crate or not at all: no `#[path]`,
# `include!` or `include_str!` that leaves a slice's folder, no symlink
# inside one. Shared *meaning* is a Concept with its fixtures, not a crate.
#
# Exits 1 on the first rule broken, every break printed as a CI annotation.
set -euo pipefail

command -v jq >/dev/null || { echo "::error::isolation.sh needs jq"; exit 1; }
command -v cargo >/dev/null || { echo "::error::isolation.sh needs cargo"; exit 1; }

# No Rust workspace yet: no slice to isolate — unless one is already there.
if [ ! -f Cargo.toml ]; then
  shopt -s nullglob
  slices=(crates/*/capabilities/*/Cargo.toml crates/*/data-capabilities/*/Cargo.toml)
  for toml in "${slices[@]}"; do
    echo "::error file=$toml::there is no root Cargo.toml — the slices are members of a workspace"
  done
  [ ${#slices[@]} -eq 0 ] || exit 1
  exit 0
fi

metadata=$(cargo metadata --format-version 1 --no-deps 2>/dev/null) || {
  echo "::error file=Cargo.toml::cargo metadata failed — the workspace does not resolve"
  exit 1
}
root=$(jq -r '.workspace_root' <<< "$metadata")

# name<TAB>relative manifest path, one line per workspace crate.
crates=$(jq -r --arg root "$root/" '.packages[] | [.name, (.manifest_path | ltrimstr($root))] | @tsv' <<< "$metadata")

# Direct path dependencies: name<TAB>dependency name, one line each. Never a
# dev-dependency: a test-only crate reaches nothing at run time.
edges=$(jq -r '.packages[] | .name as $n | .dependencies[] | select(.path != null and .kind == null) | "\($n)\t\(.name)"' <<< "$metadata")

# Direct registry dependencies, the same way: name<TAB>dependency name.
drivers='sqlx tokio-postgres postgres diesel sea-orm rusqlite mysql mysql_async'
registry=$(jq -r '.packages[] | .name as $n | .dependencies[] | select(.path == null and .kind == null) | "\($n)\t\(.name)"' <<< "$metadata")

declare -A manifest system kind
while IFS=$'\t' read -r name rel; do
  [ -n "$name" ] || continue
  manifest["$name"]="$rel"
  # The system is the first folder under crates/; the kind, where the manifest sits.
  case "$rel" in
    crates/*/capabilities/*/Cargo.toml) kind["$name"]="capability" ;;
    crates/*/data-capabilities/*/Cargo.toml) kind["$name"]="data-capability" ;;
    *) kind["$name"]="other" ;;
  esac
  if [[ "$rel" =~ ^crates/([^/]+)/ ]]; then system["$name"]="${BASH_REMATCH[1]}"; else system["$name"]=""; fi
done <<< "$crates"

declare -A direct
while IFS=$'\t' read -r from to; do
  [ -n "$from" ] || continue
  direct["$from"]+="$to "
done <<< "$edges"

# Every workspace crate reachable from a crate, transitively, space separated.
reach() {
  local -A seen=()
  local queue=("$1") current dep
  while [ ${#queue[@]} -gt 0 ]; do
    current="${queue[0]}"; queue=("${queue[@]:1}")
    for dep in ${direct[$current]:-}; do
      [ -n "${seen[$dep]:-}" ] && continue
      seen["$dep"]=1
      queue+=("$dep")
    done
  done
  echo "${!seen[@]}"
}

failed=0
fail() { echo "::error file=$1::$2"; failed=1; }

for name in "${!kind[@]}"; do
  k="${kind[$name]}"
  [ "$k" = "other" ] && continue
  what="a Capability"; [ "$k" = "data-capability" ] && what="a DataCapability"
  own="${system[$name]}"
  for dep in $(reach "$name"); do
    case "${kind[$dep]:-other}" in
      capability|data-capability)
        fail "${manifest[$name]}" "$what depends on no other slice: $name reaches $dep (${manifest[$dep]})" ;;
    esac
    if [ -n "${system[$dep]:-}" ] && [ "${system[$dep]}" != "$own" ] && [ "${system[$dep]}" != "ports" ]; then
      fail "${manifest[$name]}" "$what depends only on its own system's crates: $name ($own) reaches $dep (${system[$dep]})"
    fi
    if [ -z "${system[$dep]:-}" ]; then
      fail "${manifest[$name]}" "$what depends only on crates under crates/<system>/: $name reaches $dep (${manifest[$dep]})"
    fi
    # A crate shared inside the system is plumbing: it reaches no slice and no other system.
    for far in $(reach "$dep"); do
      case "${kind[$far]:-other}" in
        capability|data-capability)
          fail "${manifest[$dep]}" "a crate a slice depends on is plumbing, never a bridge: $dep reaches the slice $far" ;;
      esac
      if [ -n "${system[$far]:-}" ] && [ "${system[$far]}" != "${system[$dep]:-}" ] && [ "${system[$far]}" != "ports" ]; then
        fail "${manifest[$dep]}" "a crate a slice depends on stays in its system: $dep (${system[$dep]:-}) reaches $far (${system[$far]})"
      fi
    done
  done

  # Rule 4: a Capability holds no database driver of its own.
  if [ "$k" = "capability" ]; then
    while IFS=$'\t' read -r from to; do
      [ "$from" = "$name" ] || continue
      for driver in $drivers; do
        if [ "$to" = "$driver" ]; then
          fail "${manifest[$name]}" "a Capability reads through its system's read crate, never a database driver: $name depends on $driver"
        fi
      done
    done <<< "$registry"
  fi

  # Code shared outside a crate is not shared: an inclusion resolves inside the slice's folder
  # (its own manifest beside src/ is inside), and no symlink sits in it.
  dir=$(dirname "${manifest[$name]}")
  dir_abs=$(cd "$dir" && pwd -P)
  while IFS= read -r file; do
    while IFS= read -r target; do
      [ -n "$target" ] || continue
      resolved=$(cd "$(dirname "$file")" && cd "$(dirname "$target")" 2>/dev/null && pwd -P)/$(basename "$target")
      case "$resolved" in
        "$dir_abs"/*) ;;
        *) fail "$file" "$what shares code through a crate or not at all: \"$target\" leaves $dir" ;;
      esac
    done < <(grep -oE '(#\[path *= *"|include(_str|_bytes)?! *\( *")[^"]+' "$file" 2>/dev/null | sed -E 's/^[^"]*"//')
  done < <(find "$dir" -name '*.rs' -type f 2>/dev/null)
  while IFS= read -r link; do
    fail "$link" "$what shares code through a crate or not at all: a symlink inside $dir"
  done < <(find "$dir" -type l 2>/dev/null)
done

exit $failed
