#!/usr/bin/env bash
# The Rust tests a change can break, and only those — installed by
# `harness init-repo`; the repository owns it from here.
#
#   bash scripts/test-scope.sh [<base>] [-- <cargo test arguments>]
#
# <base> is what the change is measured against: the branch the pull request
# targets (`origin/milestone/...`), or a commit. Uncommitted and untracked
# files count. Without a base, the whole suite runs.
#
# A Capability crate (crates/<system>/capabilities/<name>/) depends on no
# other Capability and on nothing of the write side (docs/ARCHITECTURE.md),
# so a change that stays inside Capability crates can only break those crates
# and whatever depends on them. Then only their tests run. Anything else — a
# DataCapability, the DataGuard, an infrastructure crate, a Concept, a query,
# the CI, the toolchain — runs the whole suite.
#
# Never failing open: a path this script does not recognise widens the run,
# it never narrows it. The push that lands a milestone on `main_agent` runs
# the whole suite whatever this script says (.github/workflows/ci.yml).
#
# Prints its decision on stderr, then runs `cargo test` — or, with
# TEST_SCOPE_DRY_RUN set, prints the `cargo test` line on stdout instead.
set -euo pipefail

# run <cargo test arguments...>
run() {
  if [ -n "${TEST_SCOPE_DRY_RUN:-}" ]; then
    echo "cargo test $*"
    exit 0
  fi
  exec cargo test "$@"
}

base=""
if [ $# -gt 0 ] && [ "$1" != "--" ]; then
  base="$1"
  shift
fi
[ "${1:-}" = "--" ] && shift

# full <why> <cargo test arguments...>
full() {
  echo "test-scope: whole workspace — $1" >&2
  shift
  run --workspace --all-features "$@"
}

[ -n "$base" ] || full "no base given" "$@"
from=$(git merge-base "$base" HEAD 2>/dev/null) || full "cannot find $base" "$@"

changed=$(
  {
    git diff --name-only "$from"
    git ls-files --others --exclude-standard
  } | sort -u
)
[ -n "$changed" ] || full "no change against $base" "$@"

crates=""
while IFS= read -r path; do
  case "$path" in
    # Prose changes no test.
    *.md | docs/*) ;;
    # A crate added or a dependency taken by a Capability moves the lock;
    # the landing on main_agent runs everything against it.
    Cargo.lock) ;;
    # The root manifest, when only Capability members come and go — the
    # lines that change name a Capability, or are the list's own brackets.
    Cargo.toml)
      if git diff -U0 "$from" -- Cargo.toml | grep -E '^[+-]' | grep -vE '^(\+\+\+|---)' |
        grep -vqE 'capabilities/|^[+-][[:space:]]*(members[[:space:]]*=)?[][,[:space:]]*$'; then
        full "the root Cargo.toml changed beyond Capability members" "$@"
      fi
      ;;
    crates/*/capabilities/*/*)
      # `*` crosses `/` in a case pattern: the system is one folder, no more.
      [[ "$path" =~ ^crates/[^/]+/capabilities/[^/]+/ ]] || full "$path is outside the Capabilities" "$@"
      dir=$(echo "$path" | cut -d/ -f1-4)
      [ -f "$dir/Cargo.toml" ] || full "$dir has no Cargo.toml (removed?)" "$@"
      case " $crates " in *" $dir "*) ;; *) crates="$crates $dir" ;; esac
      ;;
    *) full "$path is outside the Capabilities" "$@" ;;
  esac
done <<<"$changed"

[ -n "$crates" ] || {
  echo "test-scope: nothing to test — only prose and the lock changed" >&2
  exit 0
}

# Each touched Capability, and every workspace crate that depends on it.
packages=""
for dir in $crates; do
  name=$(awk '/^\[package\]/{p=1; next} /^\[/{p=0} p && /^[[:space:]]*name[[:space:]]*=/{gsub(/.*=[[:space:]]*"|".*/, ""); print; exit}' "$dir/Cargo.toml")
  [ -n "$name" ] || full "$dir/Cargo.toml names no package" "$@"
  dependents=$(cargo tree --workspace --invert "$name" --prefix none --format '{p}' 2>/dev/null |
    awk '/\(\//{print $1}') || full "cargo tree cannot read $name" "$@"
  packages="$packages $name $dependents"
done
packages=$(echo "$packages" | tr ' ' '\n' | sed '/^$/d' | sort -u)

args=()
while IFS= read -r package; do args+=(-p "$package"); done <<<"$packages"
echo "test-scope: Capabilities only —" $packages >&2
run --all-features "${args[@]}" "$@"
