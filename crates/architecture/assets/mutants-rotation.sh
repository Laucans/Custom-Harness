#!/usr/bin/env bash
# The tests of the code no change touches, judged a few files at a time —
# installed by `harness init-repo`; the repository owns it from here.
#
#   bash scripts/mutants-rotation.sh <turn> [<buckets>] [-- <cargo mutants arguments>]
#
# `cargo mutants --in-diff` judges the lines a change touches; the rest of the
# code keeps whatever tests it had, and a test weakened later goes unseen.
# This script takes the files cargo-mutants would mutate (its own listing, so
# `.cargo/mutants.toml`'s exclusions hold) and deals them into <buckets>
# buckets (64 by default) by a hash of their path; turn <turn> mutates
# bucket <turn> modulo <buckets>. The CI passes its run number as <turn>.
#
# Why a hash and not a slice of the sorted list: a file's bucket depends on
# its path alone, never on the other files. Two pull requests whose branches
# have diverged — one adds files, the other removes some — still deal every
# file they share into the same bucket, so two runs with different turns
# judge disjoint files, however many run at once (up to <buckets>), and every
# file is judged once every <buckets> runs. A re-run keeps its turn and judges
# the same files again — no retrying until green.
#
# Never fails on what it finds: a missed mutant is a debt filed as an issue
# (`scripts/test-debt.sh`, run after it on the same output), not a red build
# for a change that did not make it. It fails only when cargo-mutants itself
# could not run (a usage or internal error).
#
# MUTANTS_ROTATION_LIST names a file to read the candidates from instead of
# asking cargo-mutants; MUTANTS_ROTATION_DRY_RUN prints the files picked,
# one per line, and runs nothing.
set -euo pipefail

usage="usage: mutants-rotation.sh <turn> [<buckets>] [-- <cargo mutants arguments>]"
turn="${1:-}"
case "$turn" in
  '' | *[!0-9]*) echo "$usage" >&2; exit 1 ;;
esac
shift
buckets=64
if [ $# -gt 0 ] && [ "$1" != "--" ]; then
  buckets="$1"
  shift
fi
case "$buckets" in
  '' | *[!0-9]* | 0) echo "mutants-rotation: <buckets> is a positive number, not '$buckets'" >&2; exit 1 ;;
esac
[ "${1:-}" = "--" ] && shift
bucket=$((turn % buckets))

if [ -n "${MUTANTS_ROTATION_LIST:-}" ]; then
  listing=$(cat "$MUTANTS_ROTATION_LIST")
else
  listing=$(cargo mutants --workspace --list-files)
fi

# cksum is POSIX: the same bucket for the same path on every machine.
picked=()
total=0
while IFS= read -r file; do
  [ -n "$file" ] || continue
  total=$((total + 1))
  hash=$(printf '%s' "$file" | cksum | cut -d' ' -f1)
  [ $((hash % buckets)) -eq "$bucket" ] && picked+=("$file")
done < <(printf '%s\n' "$listing" | sort -u)

echo "mutants-rotation: turn $turn, bucket $bucket of $buckets — ${#picked[@]} of $total file(s)" >&2
[ "${#picked[@]}" -gt 0 ] || exit 0
if [ -n "${MUTANTS_ROTATION_DRY_RUN:-}" ]; then
  printf '%s\n' "${picked[@]}"
  exit 0
fi

args=()
for file in "${picked[@]}"; do
  args+=(--file "$file")
done
code=0
cargo mutants --workspace --no-shuffle "${args[@]}" "$@" || code=$?
case "$code" in
  # All caught, some missed, some timed out, or the tests already failing
  # before any mutation — the last is the test job's to report, not this one's.
  0 | 2 | 3 | 4) exit 0 ;;
  *) exit "$code" ;;
esac
