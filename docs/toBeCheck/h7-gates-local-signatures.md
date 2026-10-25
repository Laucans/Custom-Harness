# H7: gates-local before the PR, and DataGuard's API in the prompt

Built 2026-10-25 on lane B (`lane-b/h3-h5-h7`, on f58d1d6 + H3, H5),
committed there with the review's fixes; not merged into `main` yet. Design, critique and decision:
`harness-dnd2-run/night/h7-design.md`, section *Decision* (the critique's
changes: an exact-string table of expressions, base verdicts from the
base's last finished gates run, S for write-side tasks too, rules-hold off,
the closure through `HARNESS_BIN`, reopened paths stay read-only).

## What shipped

- **Product assets** (`crates/architecture/assets/`, installed by
  `init-repo` as missing files): `scripts/gates-local.sh` (finds a node 22+,
  exit 3 with the fix otherwise) and `scripts/gates-local.mjs` (no
  dependency):
  - reads the product's `gates.yml` at run time, from the merged tree it
    judges (`git show <merge>:.github/workflows/gates.yml`: a gate the base
    added is run): a parser for the YAML the harness writes, an exact-string table of every `if:` and `${{ }}`
    (`CONDITIONS`, `VALUES`); an unknown one makes the step `unreadable`
    (exit 2), never skipped in silence; `github.token` and the PR number
    (without a PR) are `skip`; a job key it does not run (`needs`, `if`,
    `strategy`, `timeout-minutes`, …), a workflow key beyond name/on/jobs/
    permissions/concurrency (`env`, `defaults`) and a job or service `env`
    value it cannot give make the whole job unreadable (`--list` too, exit
    2); a step's `continue-on-error` (`tolerated`, the job stays green) and
    `timeout-minutes` are run;
  - judges the PR's tree: a snapshot of the work (real index copied:
    committed, staged, unstaged, untracked, sparse-hidden files) merged in
    memory with `origin/<base>`'s tip (`git merge-tree --write-tree`); a
    conflict is `FAIL · merge`;
  - one worktree per job under `.git/gates-local/head-<job>` (reused,
    `checkout -f`, `clean -ffdx -e node_modules`), sparse checkout off
    through `GIT_CONFIG_*` only (the session's config untouched); jobs in
    parallel, steps in order with `!cancelled()` semantics;
  - a job's `services.postgres` is a container of its own
    (`gates-local-<pid>-<side>-<job>`, `127.0.0.1::5432`, `PGPORT`
    exported, a `psql` shim that feeds `-f` on stdin when the host has no
    psql), removed on exit/INT/TERM/HUP, reaped at the next run when a
    SIGKILL left it; no docker → its psql steps `skip · no docker`;
  - a red gate is `base-red` only when the base is red there too with
    every `::error` the head reports: the base's verdict from its last
    finished `gates` run at the base tip (`gh run list/view`, annotations
    of the job) when that step concluded `success` or `failure` — a
    `skipped` pull-request-only gate of the base's push run is no verdict —,
    else a rerun of the red gates of the jobs that build
    nothing (no `cargo`) on a worktree of the base;
  - `rules-hold` off unless `--rules`; `--only a,b`, `--failed`, `--base`,
    `--label`, `--no-gh`, `--json`, `--list`; `--failed` reruns the red
    gates, the jobs whose setup failed and the closure check, everything
    after a conflict or without a report, and nothing (exit 0) only when the
    last report had nothing red and its closure held;
  - each job's gate lines printed as the job ends (`job X running…`, a red
    gate `red` until the base judged it); sessions are told a Bash timeout
    of 600000 ms (SKILL.md, CLAUDE-rules, CODE/FIX prompts);
  - the closure check through `$HARNESS_BIN architecture-check` with
    `--facts $HARNESS_FACTS` (code stage) or `--task <Closes #N>` of the
    open PR (repair); skipped and said without `HARNESS_BIN`; one that
    cannot judge (exit 2) exits 2;
  - report `.git/gates-local/report.json` (`head`, `head_tree`, `base`,
    `base_sha`, `partial` (the flags of a subset run), `gates[]`,
    `setup_failed`, `counts` (with `tolerated`, `closure-error`),
    `closure`, `conflict`); `WriteFacts` removes it before each session.
- **Harness test** `crates/architecture/tests/gates_local.rs` (18 tests):
  sync guard (all 41 step ids of `assets/gates.yml`, 0 unreadable), every
  expression in the table, uncommitted work, base-red vs one more error,
  conflict, merge with the base tip, sparse, parallel jobs (< 3.9 s for two
  2 s jobs), unreadable exit 2, no docker, closure via a fake binary, old
  node; a step the base added, `--failed` after a red closure and after a
  conflict, a closure that cannot judge (exit 2), `needs:`/workflow `env:`
  unreadable, job env unknown, `continue-on-error`, `timeout-minutes`.
- **Workflows**: `data::rust_shape` (new scan: literals emptied, multi-line
  signatures joined, traits with methods, `{ … }` on defaulted ones,
  `pub const`, `#[cfg(test)]` skipped, `Visibility::External`; the commas
  inside generics never cut a field — on the engine at 56cb6be, `Prepared`
  and `Context` show every field and no hidden count);
  `data::dependency_api` (lock parsing, `DATAGUARD_UNIT_MODULES` — lib,
  plugins, manifest, aggregate, command — first, then usage order;
  `carries`, `DEPENDENCY_BUDGET = 10_000`, footer without any path;
  proptests on the lock, the usage order, the module of a file);
  `data::facts` + `action::facts::{WriteFacts, SayGatesLocal}`;
  `checks::architecture::{judge, closure_check}` (verify unchanged);
  `CODE`, `CODE_WRITE_SIDE`, pr-fix's prompt name `scripts/gates-local.sh`;
  `Ask` carries the engine's API for write-side tasks and tasks naming it.
- **Core**: the `PUBLIC SIGNATURES` paragraph names a dependency's API.
- **Launcher**: hidden `harness architecture-check`; `HARNESS_BIN` a part
  of the `Toolchain` (its environment and its `toolchain:` line, `harness
  toolchain` too; a problem when `current_exe` fails), every session and
  process, pr-fix included; `HARNESS_FACTS` = `<loop>/<run>/session-facts.json`
  for dev-loop sessions, said once per run (`session facts: HARNESS_FACTS=…`); `dependency_index` (lock → `~/.cargo/git/checkouts/*/<rev7>`,
  else `cargo metadata --offline`).
- **Skill**: `.claude/skills/code/SKILL.md` §5 runs gates-local last before
  the PR. **Product rules**: the `CLAUDE.md` block (v8 → **v9**) says
  `bash scripts/gates-local.sh` instead of "the gates locally where they
  exist".

Measured on a copy of ForgeDesMondes `main_agent` a55bd4d (its gates.yml is
older than the asset: 33 gate steps outside `rules-hold`), `--no-gh`, with
the harness binary and a facts file of a `session.getLastBrief` task, a
neighbour's file (`list-scenes/src/lib.rs`) edited: **32 ok, 1 skip
(`rule-tests-untouched--contest`), closure FAIL naming the neighbour's
file, 212 s wall**; `manifests-conform-to-contracts` alone 186–191 s (one
`npx ajv-cli` per manifest, ~1.5 s each); `pages-mocked--bundles` 15 s;
`queries-valid` 1 s on its own container; no container left, 4 kept
worktrees, `.git/gates-local` 93 MB.

## To check

**After `harness init-repo` on ForgeDesMondes (`chore(harness): upgrade`):**

- [ ] `scripts/gates-local.sh` and `scripts/gates-local.mjs` land on
      `main_agent`; milestone branches get them through the catch-up; the
      `CLAUDE.md` block is rewritten at `<!-- harness:architecture v9 -->`.
- [ ] In a lane clone: `bash scripts/gates-local.sh --list` lists the
      product's gate steps (`grep -c '      - id:' .github/workflows/gates.yml`,
      33 today plus 5 of `rules-hold`), 0 unreadable.
- [ ] `bash scripts/gates-local.sh` (gh authenticated): verdicts gate by
      gate equal to the last `gates` run of `main_agent`
      (`gh run view --json jobs`); wall time about 3.5 min warm, the
      slowest step `manifests-conform-to-contracts` (~190 s).
- [ ] With a gate red on `main_agent`: the line says `base-red · … (gates
      run <id>)`, exit 0; with one more error of the change: `FAIL`.
- [ ] After the run: `docker ps -a | grep gates-local` empty; Ctrl-C in
      the middle: the container is removed; `kill -9` in the middle: the
      next run removes it.
- [ ] `git worktree list | grep gates-local`: the kept ones only
      (head-shell-gates, head-node-gates, head-rust-gates, closure, and
      base-* after a rerun).

**Restart the watch** (`tools/restart_watch.sh build`, with lane A's lot):
a stored prompt in `.llocal/prompts.json` overrides `CODE`/`FIX` — check it
does not hold an older `code` text without `gates-local`.

- [ ] The `toolchain:` line of the watch and of every run says
      `· HARNESS_BIN=<path of the harness binary> · PATH=…`; run.log says
      `session facts: HARNESS_FACTS=<loop>/<run>/session-facts.json`.

**Next 5 `code` sessions:**

- [ ] `session-facts.json` exists in the run's folder
      (`.llocal/loop/<run>/`), with the task's base and branch.
- [ ] The transcripts call `scripts/gates-local.sh` with a `timeout` of
      600000 (or in the background), never killed at 120 s; 0 hand-made runners
      (`grep -E "yq|yaml|sed -n '/- id:" session.log` → nothing).
- [ ] "scripts" calls per session drop from about 5 to 2 or fewer.
- [ ] run.log has `gates-local: last run on the work pushed (<tree7>) — …`
      (or `on other work than the one pushed`, or `not run`; `(partial:
      --failed)` after a rerun of the red ones): note the share.
- [ ] **ArchitectureHolds halts after a session whose last gates-local said
      `closure ok`: 0** (the L1 metric).
- [ ] A pr-fix session: its gates-local takes base, labels and `Closes #N`
      from `gh pr view`, and its closure check runs (`--task`).

**DataGuard signatures:**

- [ ] Every run start: `dataguard signatures: harness-dataguard
      dataguard-v5 @ 56cb6be — 17 module(s), carried when a task is on the
      write side or names DataGuard (budget 10000)`.
- [ ] A write-side prompt (`.llocal/logs/<run>/prompts/…`) holds
      `<signatures crate="harness-dataguard" source="dataguard-v5 @ 56cb6be"
      path="src/lib.rs">`, then plugins, manifest, aggregate, command (the
      unit's modules, always), then `api` (the next by the product's
      `harness_dataguard::` use; measured 10 052 characters with the
      footer), and the
      `async fn prepare(&self, context: &Context<'_>, …) -> Result<Prepared, Refusal>;`
      line; the footer names the modules left out and no path.
- [ ] Over the next 5 data-layer sessions: reads under `~/.cargo/git` = 0
      (baseline 5 of 8 sessions, ~39 turns); calls before the first edit
      below the baseline of ~40.
- [ ] `tools/model_ab.py` metrics unchanged or better.

**Open, not built:** step-level parallelism (decide on the per-step
timings of the reports); `manifests-conform-to-contracts` as one ajv call
(a gates.yml change, contract side); the gates' bodies into
`scripts/gates/<id>.sh` (removes the parser's drift class; after SK).
