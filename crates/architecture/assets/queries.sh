#!/usr/bin/env bash
# The registry of persisted queries (contract C) — installed by
# `harness init-repo`; the repository owns it from here.
#
#   bash scripts/queries.sh            # writes queries/registry.json
#   bash scripts/queries.sh --check    # exits 1 when the committed one differs
#
# Every read a Capability makes is a declared, persisted query: a file its
# capability.json names under `reads`, registered here under the
# Capability's id with its hash and the fields it reads. The registry is
# what tells a schema change who it affects, and what the read crate serves
# — a query not in it is not served.
#
# Where a `reads` entry is looked for, in order: at the path as written from
# the repository root, then from the Capability's own folder, then from its
# system's folder (crates/<system>/). The operation name is the first
# `query <Name>` of a GraphQL file, else the file's stem. The fields a query
# reads are declared in its first lines, as a comment:
#
#   # fields: customer.outstandingBalance, payment.dueAt     (GraphQL)
#   -- fields: customer.outstanding_balance                  (SQL)
#
# A query that declares no field is an error: the registry exists to answer
# "which Capability reads this field", and a blank answer is a wrong one.
#
# Exits 1 on a missing file or a missing fields line, every one printed as a
# CI annotation; with --check, also when queries/registry.json is not what
# this would write.
#
# Portable on purpose: the same registry under GNU and BSD tools, bash 3.2
# included — POSIX classes (`[[:space:]]`, never `\s`, which BSD sed reads as
# a literal `s`), no `globstar`, and `shasum -a 256` where `sha256sum` is
# missing.
set -euo pipefail
shopt -s nullglob

command -v jq >/dev/null || { echo "::error::queries.sh needs jq"; exit 1; }
if command -v sha256sum >/dev/null; then
  sha256() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null; then
  sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
else
  echo "::error::queries.sh needs sha256sum or shasum"; exit 1
fi

check=0
[ "${1:-}" = "--check" ] && check=1

failed=0
entries='[]'
for manifest in crates/*/capabilities/*/capability.json; do
  dir=$(dirname "$manifest")
  system=$(basename "$(dirname "$(dirname "$dir")")")
  capability=$(jq -r '.capability // empty' "$manifest")
  [ -n "$capability" ] || continue
  for read in $(jq -r '.reads[]? | if type=="string" then . else (.file // empty) end' "$manifest"); do
    file=""
    for candidate in "$read" "$dir/$read" "crates/$system/$read"; do
      if [ -f "$candidate" ]; then file="$candidate"; break; fi
    done
    if [ -z "$file" ]; then
      echo "::error file=$manifest::$capability reads $read, which is nowhere under the repository, its folder or crates/$system/"
      failed=1
      continue
    fi
    fields=$(grep -m1 -E '^[[:space:]]*(#|--)[[:space:]]*fields:' "$file" | sed -E 's/^[[:space:]]*(#|--)[[:space:]]*fields:[[:space:]]*//' | tr ',' '\n' | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' | grep -v '^$' | jq -R . | jq -s .) || fields='[]'
    if [ "$(jq 'length' <<< "$fields")" = "0" ]; then
      echo "::error file=$file::declares no \`fields:\` line — the registry answers which Capability reads which field"
      failed=1
      continue
    fi
    name=$(grep -m1 -oE '^[[:space:]]*query[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' "$file" | awk '{print $2}' || true)
    if [ -z "$name" ]; then
      name=$(basename "$file")
      name="${name%%.*}"
      name="$(tr '[:lower:]' '[:upper:]' <<< "${name:0:1}")${name:1}"
    fi
    sha=$(sha256 "$file")
    entries=$(jq -c --arg q "$name" --arg c "$capability" --arg f "$file" --arg s "$sha" --argjson fields "$fields" \
      '. + [{query: $q, capability: $c, file: $f, sha256: $s, fields: $fields}]' <<< "$entries")
  done
done
[ "$failed" = 0 ] || exit 1

registry=$(jq --argjson q "$entries" '{queries: ($q | sort_by(.capability, .file))}' <<< '{}')
if [ "$check" = 1 ]; then
  if [ ! -f queries/registry.json ]; then
    [ "$(jq '.queries | length' <<< "$registry")" = "0" ] && exit 0
    echo "::error file=queries/registry.json::missing — run scripts/queries.sh and commit it"
    exit 1
  fi
  if ! diff <(jq -S . queries/registry.json) <(jq -S . <<< "$registry") >/dev/null; then
    echo "::error file=queries/registry.json::not what scripts/queries.sh writes — a query changed, or a \`reads\` entry did: run the script and commit it"
    diff <(jq -S . queries/registry.json) <(jq -S . <<< "$registry") || true
    exit 1
  fi
  exit 0
fi
mkdir -p queries
jq . <<< "$registry" > queries/registry.json
echo "queries/registry.json: $(jq '.queries | length' <<< "$registry") quer(y/ies)"
