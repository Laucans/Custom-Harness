# Places — where everything the attendants read and write lives

Placeholders (set per factory when the briefs are built; today's values for
ForgeDesMondes on the owner's Mac):

| Placeholder | What | Today |
|---|---|---|
| `<factory>` | the factory root (the view's and the watch's cwd) | `~/.harness/factories/forgedesmondes` |
| `<harness>` | the harness checkout | `~/Workspace/Claude_code/harness/Custom-Harness` |
| `<records>` | the attendants' records folder — a human convention, not a harness concept; configurable | `~/Workspace/Claude_code/harness/harness-dnd2-run` (the "run folder") |
| `<product>` | the product repo | `Laucans/ForgeDesMondes` |

`<factory>` is a git worktree of the harness **detached at `04cb870`
(2026-10-18)**: its `CLAUDE.md`, docs and crates are stale. Read the harness
in `<harness>`.

## Clocks

| Source | Clock |
|---|---|
| `watch.log`, `errors.tsv`, `costs.tsv`, `harness.db` (`at`), `night/sonnet-switch-at.txt` | UTC (`Z`) |
| `watch.out` | no time at all — read times in `watch.log` |
| the follower's lines, `db-sweeper.log`, the machine (`date`) | local = Paris |
| `tools/costs.py` | hard-codes UTC−4: wrong, convert |
| the records (`DECISIONS.md`, incidents, priorities, reports) | **Paris**, written `HH:MM Paris` |

Paris = UTC+2 until 2026-10-25 03:00, then UTC+1. `DECISIONS.md`: its
10-18…10-20 rows are `Z` (header "date UTC"), entries since 10-23 23:05 Paris.

## The harness (`<harness>`, branch `main`, never pushed without the owner's word)

| Path | What |
|---|---|
| `CLAUDE.md` | the rules every change is checked against (hexagonal, lints, tests, workflow) |
| `ARCHITECTURE_OVERVIEW.md`, `crates/<crate>/ARCHITECTURE.md` | the split and each crate's design — read the crate's before touching it |
| `docs/toBeCheck/*.md` | what shipped without a real run: context, commits, checklist; tick with the time seen, delete when complete |
| `docs/MIGRATION.md`, `docs/PERSISTENCE.md`, `.claude/skills/` | migration decisions, Docker persistence, the skills the factory's sessions run |
| `crates/architecture/assets/` | every file `init-repo` installs in a product (gates.yml, scripts, CLAUDE-rules.md, mock server, test-kit…) and the dev loop's skeletons |
| `crates/workflows/src/common/{labels,review,decision}.rs` | the labels, markers and `## Decision` fields below, authoritative |

## The factory (`<factory>`)

| Path | What |
|---|---|
| `.env.local` | the watch's environment — see *`.env.local` keys* |
| `.llocal/logs/agent-loop/watch.out` | the watch's stdout/stderr (`restart_watch.sh` appends to it), **no timestamps**: `tick:`, `idle:`, `watch: lanes -> …`, catch-ups, merges, `warning:` lines |
| `.llocal/logs/agent-loop/watch.log` | the same lines stamped `[YYYY-MM-DDTHH:MM:SSZ]` (UTC), plus `saw: … dev_loop=[…]` and `schedule:` per tick. Read times here; the follower reads this file |
| `.llocal/logs/agent-loop/errors.tsv` | one row per stop: `when` (UTC), run, workflow, kind, reason |
| `.llocal/logs/agent-loop/costs.tsv` | the ledger: every paid session (cost, turns, outcome, milestone column); `costs.tsv.bak` = the copy before the 10-24 backfill |
| `.llocal/logs/agent-loop/quota.json` | the Claude quota windows (`five_hour`, `seven_day`: `utilization`, `resets_at`) the watch rests on |
| `.llocal/logs/agent-loop/cmd-<args>.out`, `route-<kind>-<n>.out` | what `tools/harness-cmd.sh` and `tools/route.sh` printed |
| `.llocal/logs/<workflow>/<run-id>/` | `run.log`, `session.log`, `stream.jsonl` (model, turns, cost in the `result` line). Workflows: agent-loop, pr-review, pr-fix, decide, rules, split, planner, refinement, contract-manager, milestone-merge, main-agent-merge, test-debt, plan-graph, data-diagnosis |
| `.llocal/harness.db` | SQLite event log (`Logbook::event`), `at` in UTC; read with `sqlite3 -readonly` |
| `.llocal/lanes/lane-N.log`, `.llocal/agentic_workspaces/` | each lane's log; the router's checkouts (`router-lane-N`, `router-rules`, `catch-up` …) |
| `.llocal/toolchain/bin` | symlinks first on the sessions' PATH: bash 5.3, GNU sed (`gsed`) and grep (`ggrep`), node 26. No cargo here: cargo is the product's pinned rustup toolchain, next on PATH (`~/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin`), recorded at each start as a `toolchain_resolved` event |
| `.llocal/prompts.json`, `.llocal/prompts.drafts.json` | the prompt book (harness-written) and the view's drafts. **Absent in this factory** until a draft is saved; a watch start applies the drafts |
| `.llocal/*-locks/`, `watch-lock` | per-route locks; never delete while the watch runs |
| `~/.harness/factories/harness-workspaces/lane-N` | the lanes' product checkouts **with their own `target/` (~3–8 GB each): the first place the disk goes** |

### `.env.local` keys

Never print the file: list keys with `cut -d= -f1 .env.local`, read one key
with `grep -o '^KEY=[^ ]*'`, never `DATABASE_URL`. Every change is logged in
`DECISIONS.md` with its old value, and takes effect at the next watch start.

| Key | Meaning |
|---|---|
| `HARNESS_WATCH_COMMAND` | `<records>/bin/harness watch --force-reset --no-human-gate --parallel 5 --interval 60` — lanes and interval change here; the line `restart_watch.sh` pgreps |
| `MODEL`, `EFFORT` (empty = defaults), `DATA_LAYER_MODEL`, `DATA_LAYER_EFFORT`, `STEWARD_MODEL` | models and efforts per stage |
| `BUDGET_FLOOR` | USD floor of a task's budget (default 6, here 12) — **not** a disk floor |
| `IGNORE_QUOTA` | `true`: no 90 % Claude reserve |
| `PERMISSION_MODE` | `bypassPermissions`, inherited by the view's desks |
| others | loop shape (`MAX_ROUNDS`, `NO_NARROW`, `STAGES`), where work happens (`INTEGRATION_BRANCH`=`main_agent`, `WORKSPACE_*`, …), the view (`HARNESS_BIN`, `HARNESS_VIEW_PORT`=7878, but the running view is on **7879**) |
| `DATABASE_URL` | **secret** |

## The product (GitHub `<product>`)

- Branches: `main` (release: `main_agent_merge` opens and merges the PR from
  `main_agent`), `main_agent` (integration), `milestone/<n>-<slug>` (one per
  building milestone), `archive/milestone/*` (frozen, never touched), task
  branches `feat/…`, rules branches `rules/<issue>`, `design-system` (the
  kit, contract O).
- Labels: the full list is `labels.rs` (side labels in
  `harness-architecture::vocabulary`); `harness:parked` and
  `harness:blocked` do not exist. Semantics that matter:
  - `harness:pr-fix` authorises one repair, consumed before paying.
  - `harness:needs-decision` → `decide` takes it at the next tick.
  - `harness:ai-decided` is read by no workflow: taking it off means "seen".
  - `harness:triggered` removed by hand asks for a resplit.
  - `harness:review-pending` on a task is what the merge train looks for.
  - while a `harness:rules` task is open, no development starts.
- Markers (`review.rs`): `<!-- agent-review -->` opens every review, with
  `<!-- agent-review-verdict: clean|blocking -->`; `<!-- agent-fix -->` on
  every repair (cap 2 since the last `<!-- harness-decided: repair -->`);
  `<!-- harness-decided: merge -->` counts only beside `agent-review` and a
  clean verdict; `<!-- rules-fidelity: faithful|unfaithful -->` on rules PRs.
- `## Decision` lines (`decision.rs`): `kind`, `blocks` (required — without
  it there is no question), `needs`, `produces`, `cut`, `round`,
  `decided-by` (`harness`, `ai`, `owner`), `verdict`. `decide` never
  decides over `decided-by: owner`.
- A task's `## Architecture` section: `unit:`, `system:`, `produces:`,
  `needs:`, `side:` — the closure, the narrowed checkout and the scaffold
  read them there.
- `Closes #N` alone on its line in a PR body, or the task never closes;
  `CONTEST.md` on a contesting branch.
- **Contract change issue** (what the dev loop opens): title `Contract change:
  <id> for #<task>`, body lines `id:`, `task: #N`, `milestone: #M`, `need:`,
  label `harness:contract-change`, the task `blocked_by` it.
- Product files that matter: `plan/graph.json` (contract R), `rules.json`,
  `tests/rules/`, `mock/` (server, handlers, pages.json, bundles),
  `crates/*/data-capabilities/*/data-capability.json`, `apps/`.
- GitHub rate limit (5 000/h) is shared with the watch: no tight `gh` loops,
  poll at ≥ 30 s, `gh api rate_limit` when in doubt.

## The records folder (`<records>`) — the attendants' own records

| Path | What |
|---|---|
| `DECISIONS.md` | every autonomous choice: time (Paris), choice, why, alternatives, how to undo — the owner's report |
| `night/report-<date>.md`, `night/tomorrow.md` | the morning report; the next day's plan |
| `night/priorities-H.md` | every parked item, scored 1–10, with an ID (H1…, R, P, D, M…) and the owner's rulings |
| `night/<lot>-design.md`, `night/<lot>-critique.md` | designs and critiques before a lot is built (decision appended) |
| `night/incidents/` | the watcher's hand-overs (shape: `watcher.CLAUDE.md`, step 5) and the fixer's restart checklists |
| `night/dev-plan.md` | present while a harness development plan holds the factory stopped (`rulings.md`); written and removed by the fixer |
| `night/snapshots.jsonl` | the metrics snapshots (`tools/night_metrics.py`) |
| `night/model_ab.json`, `night/sonnet-switch-at.txt` | the model comparison and its cut-over time (UTC) |
| `night/*.log`, `view.log` / `view.pid`, `disk-cleaner.log` | what the tools printed (restarts, installs, db-sweeper, the view on :7879, disk-cleaner) |
| `bin/harness`, `bin/harness-view` | the installed binaries: `tools/restart_watch.sh build` swaps `bin/harness` only; `tools/start_view.sh build` swaps `bin/harness-view` and restarts the view |
| `tools/` | the instruments (`playbooks.md`, *Instruments*) |
| `watch.log`, `watch.pid`, `loop.pid`, `disk-cleaner.pid` | **stale** (Oct 8–20): find live processes by pattern (`playbooks.md`, the `pgrep` trap) |

## Scratch and services

- Your session scratchpad: product clones, `CARGO_TARGET_DIR`s — removed
  when done (a target grows past 40 GB). `$TMPDIR/cargo-mutants-*`:
  removed when no mutants run owns them. `~/.harness/sccache`: unused by
  lanes until lot D. Docker (OrbStack): `harness-postgres`
  (`docker/postgres/compose.yml`, `scripts/postgres.sh`).
