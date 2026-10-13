#!/usr/bin/env bash
# Prompt-cache report per stage, from a spending ledger (costs.tsv).
#
# A cache write costs twice the input price (1-hour TTL), a read a tenth:
# a stage whose `write%` is high pays for context it could have re-read.
# See docs/SESSION-CARRIER.md, "Prompt cache".
#
# Usage: scripts/cache-report.sh [costs.tsv ...]
#        (default: .llocal/logs/agent-loop/costs.tsv)
set -euo pipefail

if [ "$#" -eq 0 ]; then
  set -- .llocal/logs/agent-loop/costs.tsv
fi

awk -F'\t' '
  FNR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
  {
    s = $col["stage"]
    n[s]++
    usd[s] += $col["cost_usd"]
    rd[s] += $col["cache_read"]
    wr[s] += $col["cache_write"]
    all_rd += $col["cache_read"]; all_wr += $col["cache_write"]; all_usd += $col["cost_usd"]; rows++
  }
  END {
    printf "%-22s %5s %9s %8s %10s %10s\n", "stage", "n", "usd", "write%", "write/n", "read/n"
    sorted = "sort -k3 -nr"
    for (s in n) {
      t = rd[s] + wr[s]
      printf "%-22s %5d %9.2f %7.1f%% %10d %10d\n", s, n[s], usd[s], (t ? 100 * wr[s] / t : 0), wr[s] / n[s], rd[s] / n[s] | sorted
    }
    close(sorted)
    t = all_rd + all_wr
    printf "%-22s %5d %9.2f %7.1f%%\n", "TOTAL", rows, all_usd, (t ? 100 * all_wr / t : 0)
  }
' "$@"
