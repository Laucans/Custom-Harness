#!/usr/bin/env sh
# Bundles the mock-up's micro-frontends from the real Micro-UIs' code and
# records what each was built from — see mock/build.mjs. Run from the
# repository root after a change under apps/ or mock/src/; the bundles are
# committed so the mock-up needs no build to be viewed.
set -eu
cd "$(dirname "$0")/.."
node mock/build.mjs
