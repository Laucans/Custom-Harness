# Prompt cache: refinement fork and lean robot sessions

**Status: merged on `main`, not yet verified in a real run.**

## Context

Robot sessions (`claude -p`) wrote far more prompt cache than they read. Every
refinement step opened a fresh session and re-wrote the repo map and the
hierarchy (~30k tokens) at the 1-hour TTL, which costs twice the input price.
In the last measured run, that was 54% of all cache writes.

Measured on 2026-10-13 (sonnet, "reply OK" calls in lane-0):

| Refinement step | Cost |
|---|---|
| Fresh session | $0.129 |
| Fork of the primed session | $0.037 (−71%) |

The design and the measurements are in `docs/SESSION-CARRIER.md`, section
"Prompt cache".

## What shipped

- `3e07156`: robots start with `--strict-mcp-config --setting-sources
  project,local --exclude-dynamic-system-prompt-sections`. No operator MCP
  servers, user plugins or user skills; project skills still load.
- `f11ca4b`: a `prime` stage tells the map and the hierarchy once, right
  after `explore`. Every later refinement step forks it
  (`--resume <base> --fork-session`) and sends only its template. The
  adapter takes the base's cost out of a fork's reported cost.
- `eef287b`: when a planner or split reply's JSON does not parse, the retry
  sends only the error, not the whole prompt again.

## To check in a real run

1. Run a refinement on the latest `main` (e.g. dnd_helper2), then
   `bash scripts/cache-report.sh <path>/costs.tsv`:
   - `write%` below 15% on business-goal, acceptance-criteria,
     business-rules, coherence and human-advice (was 42–62%);
   - `write/n` on business-goal divided by at least 3 (was ~27k).
2. A `prime` row appears once per refinement run in `costs.tsv`.
3. Fork costs are not double-counted: a forked step's `cost_usd` is cents,
   not the base's cost plus its own.
4. The written sections are as good as before. A fork still sees the map,
   the hierarchy and its template, but across two messages.
5. Robots still find the project skills (`/tech-analyst`, `/code`,
   `/create-test`) and authenticate without the user settings.
6. A planner/split JSON retry, if one happens, still produces a valid plan.

## Open follow-up

Dev loop: run `technical-refinement` on sonnet and fork `code` from its
session, behind a `BRANCH_CODE` switch. Not started. It needs a rewritten
`code` instruction, and the base cleared after `code` so `create-test` does
not fork.

When every point above is checked, delete this file.
