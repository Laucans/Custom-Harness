#!/usr/bin/env bash
# The dead code the compiler cannot see — installed by `harness init-repo`;
# the repository owns it from here.
#
#   bash scripts/dead-code.sh
#
# rustc's `dead_code` lint stops at the crate's edge: a `pub` item is an API,
# and whether anyone in the workspace uses it is not the compiler's question.
# In a workspace of library crates built for one or two binaries it is the
# only question. This script asks it the cheap way: every `pub` function,
# type, trait, constant or static declared under a `src/` directory whose
# name is written nowhere else in the repository's Rust sources — not in a
# call, a `use`, a test, a doc comment — is dead, and listed. One finding is
# enough to fail (exit 1).
#
# What it does not see, on purpose:
# - a name used only by the module that declares it, if that use is on
#   another line: that is not dead, and rustc judges the rest;
# - a `pub mod`: a module reached through `pub use m::*` is used without
#   its name ever being written again;
# - an export — `#[wasm_bindgen]`, `#[no_mangle]`: its caller is the page or
#   the linker, which the sources do not show;
# - an item kept for a reason the repository does not show — an API a
#   downstream repository calls, a hook a macro fills in. Say so where rustc
#   expects it, in the attributes above the item: `#[allow(dead_code)]`
#   (with the reason in a comment) and this script passes it too.
#
# Unused dependencies are the same question one level up, and `cargo
# machete` answers it; the CI runs both.
set -euo pipefail

# The Rust sources, tracked ones only: a build directory is not a user.
sources=$(git ls-files -- '*.rs')
[ -n "$sources" ] || { echo "dead-code: no Rust source" >&2; exit 0; }

# Where an item may be declared: under a `src/`, never `main.rs`, whose
# items are the compiler's to judge, and never a `bin/`.
declaring=$(printf '%s\n' "$sources" | grep -E '(^|/)src/' | grep -vE '(^|/)(main\.rs|bin/)' || true)
[ -n "$declaring" ] || exit 0

# One pass over everything: how many times each identifier is written.
counts=$(mktemp)
trap 'rm -f "$counts"' EXIT
# shellcheck disable=SC2086
grep -ohwE '[A-Za-z_][A-Za-z0-9_]*' $sources | sort | uniq -c > "$counts"

# shellcheck disable=SC2086
findings=$(awk -v counts="$counts" '
  BEGIN {
    while ((getline line < counts) > 0) {
      split(line, f, " ")
      seen[f[2]] = f[1] + 0
    }
    close(counts)
  }
  FNR == 1 { kept = 0 }
  {
    line = $0
    if (match(line, /^[ \t]*pub(\([a-z]+\))?[ \t]+((async|unsafe|const)[ \t]+)*(fn|struct|enum|union|trait|type|const|static)[ \t]+[A-Za-z_][A-Za-z0-9_]*/)) {
      decl = substr(line, RSTART, RLENGTH)
      n = split(decl, words, /[ \t]+/)
      name = words[n]
      if (!kept && seen[name] <= 1) {
        printf "%s:%d: pub %s is written nowhere else\n", FILENAME, FNR, name
      }
      kept = 0
    } else if (line ~ /allow\(dead_code\)|wasm_bindgen|no_mangle/) {
      # The allowance holds through the attributes and docs down to the item.
      kept = 1
    } else if (line !~ /^[ \t]*(#\[|#!\[|\/\/)/) {
      kept = 0
    }
  }
' $declaring)

if [ -n "$findings" ]; then
  printf '%s\n' "$findings"
  echo "dead-code: $(printf '%s\n' "$findings" | wc -l | tr -d ' ') pub item(s) the workspace never uses" >&2
  exit 1
fi
echo "dead-code: every pub item is used" >&2
