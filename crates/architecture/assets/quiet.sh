#!/usr/bin/env bash
# A check that costs one line when it passes and every line of its errors
# when it fails — installed by `harness init-repo`; the repository owns it
# from here.
#
#   bash scripts/quiet.sh <command> [<arguments>...]
#
# Runs the command with its stdout and stderr captured together, in order.
#
# Passed (exit 0): one line, `ok · <command> · <seconds>s`. Nothing else.
#
# Failed: every line of the output except the ones known to say only that
# something is progressing or passed — cargo's `Compiling`/`Checking`/...
# lines, `test <name> ... ok` and `... ignored`, and the blocks of test
# binaries that passed whole (`Running`, `running N tests`, `test result:
# ok.`). Nothing is cut short: a line that is not one of those is printed as
# it came, an error, a warning, a panic, a diff, the `left`/`right` of an
# assertion, the list of failures. When the filter leaves nothing, the whole
# output is printed. The full log is kept, its path on the last line, and the
# command's exit code is this script's.
set -uo pipefail

if [ $# -eq 0 ]; then
  echo "usage: bash scripts/quiet.sh <command> [<arguments>...]" >&2
  exit 2
fi

log=$(mktemp "${TMPDIR:-/tmp}/quiet.XXXXXX")
start=$(date +%s)
"$@" >"$log" 2>&1
code=$?
seconds=$(($(date +%s) - start))

if [ "$code" -eq 0 ]; then
  rm -f "$log"
  echo "ok · $* · ${seconds}s"
  exit 0
fi

# A block opens on a test binary's `Running`/`Doc-tests` line and is printed
# only when one of its lines says more than "passed".
kept=$(awk '
  function noise(line) {
    return line ~ /^[[:space:]]*$/ \
      || line ~ /^running [0-9]+ tests?$/ \
      || line ~ /^test .* \.\.\. (ok|ignored)$/ \
      || line ~ /^test result: ok\./
  }
  function flush() {
    if (open && useful) printf "%s", block
    block = ""; open = 0; useful = 0
  }
  /^[[:space:]]+(Running|Doc-tests) / { flush(); open = 1; block = $0 "\n"; next }
  /^[[:space:]]+(Compiling|Checking|Documenting|Downloaded|Downloading|Updating|Locking|Adding|Fresh|Blocking|Finished|Packaging|Verifying|Archiving) / { next }
  open {
    if (noise($0)) { if ($0 !~ /^test .* \.\.\. (ok|ignored)$/) block = block $0 "\n"; next }
    useful = 1; block = block $0 "\n"; next
  }
  { print }
  END { flush() }
' "$log")

if [ -n "$(printf '%s' "$kept" | tr -d '[:space:]')" ]; then
  printf '%s\n' "$kept"
else
  cat "$log"
fi
echo "failed (exit $code) · $* · ${seconds}s · full log: $log"
exit "$code"
