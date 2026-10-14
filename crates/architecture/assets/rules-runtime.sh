#!/usr/bin/env bash
# The rules' runtime: how the rules' tests (tests/rules/, contract P) reach
# the product — by identifier, over the shell's wire protocol, as a Micro-UI
# does. Installed by `harness init-repo` as a stub that serves nothing: every
# rule's test stays pending. The task that builds the product's server (or
# its shell's host) teaches it to serve, and keeps it serving.
#
#   bash scripts/rules-runtime.sh serve
#       Starts the product for the tests — its server on the store with the
#       mock-up's fixtures loaded, its screens if it has any — in the
#       background, and prints the variables the tests read, one KEY=value a
#       line (the CI appends them to $GITHUB_ENV):
#         RULES_SERVER_URL=http://127.0.0.1:<port>    the wire protocol's root
#         RULES_APP_URL=http://127.0.0.1:<port>       where the screens are
#         RULES_SCREEN_ROUTE=/screens/<screen>        a screen's route
#       Prints nothing while nothing can be served: the tests stay pending.
#
#   bash scripts/rules-runtime.sh stop
#       Stops what serve started.
set -euo pipefail

case "${1:-}" in
  serve) exit 0 ;;
  stop) exit 0 ;;
  *)
    echo "usage: bash scripts/rules-runtime.sh serve | stop" >&2
    exit 2
    ;;
esac
