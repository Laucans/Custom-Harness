#!/usr/bin/env bash
# Every gate of .github/workflows/gates.yml on a full copy of your work,
# before the pull request — installed by `harness init-repo`; the
# repository owns it from here. The engine is scripts/gates-local.mjs (its
# header says what it runs and how it judges); this entry only finds a node
# that runs it.
#
#   bash scripts/gates-local.sh             every gate, jobs in parallel, plus
#                                           the closure check in a harness session
#   bash scripts/gates-local.sh --failed    only the gates red last time
#   bash scripts/gates-local.sh --only a,b  only these gates
#   bash scripts/gates-local.sh --rules     rules-hold too (starts the product)
#
# Exit 1: a gate or the closure check failed. 2: a step it could not read.
# A `base-red` gate is red on the base too, with the same errors: not yours.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
node_bin=${GATES_LOCAL_NODE:-node}
major=$("$node_bin" -p 'process.versions.node.split(".")[0]' 2>/dev/null || echo 0)
if [ "$major" -lt 22 ]; then
  echo "gates-local: node 22 or later is needed (the gates' own scripts run on it); found $("$node_bin" --version 2>/dev/null || echo none) at $(command -v "$node_bin" || echo nowhere) — put a node 22+ first on PATH (e.g. /opt/homebrew/bin) or set GATES_LOCAL_NODE" >&2
  exit 3
fi
exec "$node_bin" "$here/gates-local.mjs" "$@"
