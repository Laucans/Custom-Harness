#!/usr/bin/env bash
# The mutants no test caught, filed as issues an agent closes — installed by
# `harness init-repo`; the repository owns it from here.
#
#   bash scripts/test-debt.sh <mutants.out directory> [<where it was found>]
#
# Reads `missed.txt` of a cargo-mutants run (`scripts/mutants-rotation.sh`'s)
# and files one issue per source file, labelled `harness:test-debt`, titled
# `Test debt: <file>`. A file that already has one open — labelled or not —
# gets a comment listing what is missed now and the label again, never a
# second issue. The harness's `test_debt` workflow takes a labelled issue,
# removes the label as it starts (one label, one attempt), and writes the
# tests that catch those mutants; a file still owing tests is asked again
# here, the next time the rotation judges it.
#
# Needs `gh`, authenticated for the repository (GH_TOKEN in CI). Creates the
# label when it is missing, without touching it when it exists.
set -euo pipefail

LABEL="harness:test-debt"

out="${1:-}"
[ -n "$out" ] || { echo "usage: test-debt.sh <mutants.out directory> [<where>]" >&2; exit 1; }
where="${2:-a mutation run}"
missed="$out/missed.txt"
if [ ! -s "$missed" ]; then
  echo "test-debt: no missed mutant" >&2
  exit 0
fi

gh label create "$LABEL" --color 5319e7 \
  --description "Code cargo-mutants can break without a test failing" >/dev/null 2>&1 || true

# One file at a time: its lines of missed.txt, `path:line:col: description`.
for file in $(cut -d: -f1 "$missed" | sort -u); do
  title="Test debt: $file"
  mutants=$(grep -F "$file:" "$missed" | sed -E "s|^$file:([0-9]+):([0-9]+): (.*)$|- line \1: \3|")
  n=$(printf '%s\n' "$mutants" | wc -l | tr -d ' ')
  # Any open issue of that title: the workflow took the label off it.
  existing=$(gh issue list --state open --search "\"$title\" in:title" \
    --json number,title --jq ".[] | select(.title == \"$title\") | .number" | head -n 1)
  if [ -n "$existing" ]; then
    gh issue comment "$existing" --body "Still missed in $where — $n mutant(s):

$mutants" >/dev/null
    gh issue edit "$existing" --add-label "$LABEL" >/dev/null
    echo "test-debt: $file — commented on #$existing and asked again ($n)" >&2
    continue
  fi
  gh issue create --label "$LABEL" --title "$title" --body "\`cargo mutants\` rewrote these parts of \`$file\` and every test still passed — $n mutant(s), found in $where:

$mutants

## What to do

The harness's \`test_debt\` workflow takes this issue while it carries \`$LABEL\` — or a human, with \`/create-test\`. Write the tests that catch them: each test fails on its mutant and passes on the code as it is — prove it red, then green. Never change the code to dodge a mutant. A mutant with no observable difference (an equivalent one) goes in \`.cargo/mutants.toml\`'s \`exclude_re\`, with the reason beside it.

## Verification

- \`cargo mutants --workspace --file $file\` reports no missed mutant.
- The rest of the suite still passes." >/dev/null
  echo "test-debt: $file — issue opened ($n)" >&2
done
