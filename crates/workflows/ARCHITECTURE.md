# `harness-workflows` — architecture

The instances: what `harness-core` cannot name. Eight workflows declared
against the framework, seven deterministic commands beside them (§13), plus
`common/` for what at least two of them actually read.

Read this before adding a workflow or moving a file inside one. The crate graph
is in [ARCHITECTURE_OVERVIEW.md](../../ARCHITECTURE_OVERVIEW.md); the framework
itself in [../core/ARCHITECTURE.md](../core/ARCHITECTURE.md); the repo's Rust
rules (lints, errors, `?Send`, docs on every `pub`) in `../../CLAUDE.md`.

Each rule below is here because its absence cost something. When a rule gets in
the way, read its reason before working around it.

## 1. The skeleton

A workflow is a directory, and it always has this shape:

```
<workflow>/
  mod.rs            the summary, and nothing else
  ports.rs          the ports: Rc<dyn Trait>, never a value
  config.rs         the settings: values, never a port
  run.rs            the assembly: Request + build()
  data/             what it reads and decides — pure business logic
  action/           what it writes
  checks/           what judges, without ever writing
  orchestration/    what sequences: stages.rs, round.rs, workflow.rs
```

The split is by **role**, not by subject. A file finds its place by answering
one question: *does it read, write, judge, or sequence?*

- `data/` — the state (the `S` of `Context<S>`) and the business rules that have
  neither I/O nor subprocess. A policy is tested here without network and
  without doubles.
- `action/` — everything that writes: an `Action`, a `SessionAction`. Also where
  the "writes" half of a split guard lives (rule 6).
- `checks/` — the `Verification`s. A stage's, not the launcher's: those belong
  to no workflow.
- `orchestration/` — `stages.rs` (the table), `round.rs` (the mounted round) and
  `workflow.rs` (the executable shape). Nothing else goes in.

`ports.rs`, `config.rs` and `run.rs` stay at the root because **none of the
three is a step**: the first two are the contract the launcher fulfills, the
third is the entry point that, with that contract filled, assembles the whole
workflow.

**A deterministic command departs from this skeleton**: no `orchestration/`,
no `checks/`. `init_repo`, `main_agent_merge` and `data_snapshot` are all this, not a
workflow — no `Context`, no stage table, no session, no cost — so a stage
table would name one entry for a sequence that does not exist, and a
`Verification` would judge against a `Context<S>` nobody carries. What
`checks/` would hold (a read-only audit's criteria, a merge's readiness
predicate) lives in that module's own `data::audit` as pure functions over
values already read. See `init_repo/mod.rs` and `main_agent_merge/mod.rs`
for the same reasoning in place — the router (`common::routing` +
`harness watch`) is this too, though its own "audit" is a pure decision
function rather than a module of its own.

## 2. Ports ≠ Config

Two objects, never one:

- **`Ports`** carries what is **injected** — `gh`, `sessions`, `spending`,
  `locks`, `repo`, `disk`. What a test replaces with a fake.
- **`Config`** carries what **this run is worth** — a branch, a directory, a
  `SessionSpec`, a flag.

A single type for both existed (it was called `Wiring`) and was split: it
answered two different questions — "what is it made of" and "what does this run
change" — under one name, so no name was right.

The criterion is **nature, not form**: `now: fn() -> String` is a function
pointer and lives in `Ports`, because it is the injected clock — neither core nor
workflows carry a dependency on time.

`Config` is also distinct from core's `Settings`: `Settings` carries what
**every** workflow has (`--dry-run`, `--stages`) and core reads it from the
`Context` at every stage. `Config` is read once, at assembly.

## 3. The direction of dependencies

Outside tests, a `use crate::<workflow>::…` only goes this way:

```
data  ←  checks
data  ←  action
data, action, checks, ports, config  ←  orchestration
everything else  ←  run
```

That is: `data/` knows nobody, `checks/` and `action/` know only `data/`,
`orchestration/` knows everything but `run`, and **nothing depends on `run`**. A
`use` that climbs means a file is in the wrong place, not that an exception is
needed.

Checkable in one command:

```
grep -rn "^use crate::" <workflow>/{data,checks,action}/ | grep -v "::data\|common::"
```

## 4. Stage names are written in the table

A stage name (`"code"`, `"inline"`, `"router"`) is written in
`orchestration/stages.rs` and **descends as a field** to the gate or action that
needs it.

Never a constant shared between two layers, never a repeated literal. The
failure mode avoided: a gate looks in `ctx.results` for a stage's reply under a
name nobody wrote, doesn't find it, and concludes the stage was skipped — when
it ran, and was paid for.

## 5. Text lives with the table; the action receives it

Prompt templates live in `orchestration/` — in `stages.rs` when there are few,
in `prompts.rs` when there are seven. The action receives them as a field
(`template: String`): the table resolves it as `config.prompts.text(KEY,
DEFAULT)` — the human's edit from the view, or the constant. Every editable
prompt is named once in the crate-level `prompts` catalog (key, stations,
default), which is what `harness-view` lists and edits.

An action that went up to read them would invert the direction of composition
(rule 3), and the workflow would have two design surfaces instead of one.

## 6. Judging is not doing

A `Verification` **judges and does not write** (migration decision #1), and the
borrow checker holds it — it only ever receives `&Context`.

So a Python guard that wrote splits in two:

| what it did | what it becomes |
| --- | --- |
| re-reads, places a label, mutates state | a `Verification` in `checks/` **+** an `Action` in `action/` |

The price is one re-read per pair — a free local call against a session that
costs dollars.

## 7. The shape on one side, its assembly on the other

- `orchestration/workflow.rs` carries **the type** and its impls (`Workflow`,
  `Executable`). It names neither `Ports`, nor `Config`, nor `Request`.
- `run.rs` carries **the assembly**: a `Request` (what the invocation asks for,
  distinct from infrastructure) and a `build()` returning the mounted type.

What that buys: the shape is tested alone (remaining rounds, tolerance, lock)
without mounting the real table, and the wiring is tested alone (which field
receives what) without executing anything. The two files change for different
reasons — one when the workflow's semantics change, the other when the wiring
does.

**`run.rs` builds no concrete adapter — and neither does an action or a gate.**
Naming a `GhCli`, a `GitCli`, a `ClaudeCliFactory`, a `Checkpoint` or a
`ReviewLedger` is the launcher's job, and its alone: that is what leaves each
port a single test seam. A workflow imports from `harness_core::ports`, and
`harness-adapters` is only a dev-dependency of this crate, so shipped code
cannot name an adapter at all. It carries no `std::fs` call and no clock of
its own — if what it needs has no port yet, the port is what gets added
(`Checkpoints` and `ReviewCosts` were added exactly that way). The rule and its
test-only exception (`Rehearsal`) are in `../../CLAUDE.md`.

## 8. One design surface, and only one

`orchestration::stages::table` **is** the sequence: the order of its entries is
the execution order, adding an entry adds a stage, removing one removes it.

This wasn't always true — the sequence used to live in a graph's `@listen`
decorators, and the table was only a registry of settings that could be reversed
entirely without execution changing. Any return to a declarative layer above the
table walks that road backwards.

Two accepted cases stay out of the table, each documented where it lives: a
**branch** (`dev_loop`'s rollover, which leaves when the sequence has nothing to
do) and **shared entries** (the repo map, which
`refinement::orchestration::round` puts in front — `stages.rs` has no way to read
the repo, and has no business having one). Both live in `round.rs`: the round
carries the order, and the assembly does not rewrite it.

## 9. The fakes

Every `ports.rs` and every `config.rs` carries its `#[cfg(test)] pub(crate) mod
fake`:

- `ports::fake::ports()` / `ports::fake::with(gh)`;
- `config::fake::config()`.

A test port **refuses** rather than doing nothing: mounting a table must open no
session and record nothing, and a silent port would let the opposite through
unnoticed. The exception is documented where it is taken — `refinement`
rehearses its sessions (`Rehearsal`) because its preflight tests run the whole
sequence in dry-run.

A fake is never invented at the call site: a fake **adapter** is injected. A
test that passes against a mock while the real adapter is broken is exactly the
failure mode this avoids.

## 10. The tests

- A test lives **with its subject**, in the module's `#[cfg(test)] mod tests`.
- A test may climb the stack — calling `run::build` to mount its subject is
  normal. Rule 3 is about production code.
- What is tested in `orchestration/stages.rs` is **order**; in `run.rs`, the
  **wiring**; in `data/`, the **policy**. A business rule is not tested through a
  mounted table.

## 11. `common/`

Only what **at least two** workflows actually need today goes in — not what might
one day serve a third.

`common/` does not take the four directories: these are not workflows. But rule
2 still holds — `explore` has its own `Ports` and `Config`.

Current contents:

| module | what it is |
| --- | --- |
| `labels` | the `harness:*` labels, created by `init-repo`; each preflight checks they exist, because a misspelled label makes a list empty and an empty list reads as "nothing left to do". Three of them say which side of the agent-native architecture a task is on: `read-side` (runs in parallel, merges alone), `write-side` (runs alone; its PR merges once reviewed and green), `review-pending` (that PR is open and waits for its review and its checks). |
| `sections` | the canonical sections of an issue body. `split` writes `Scope` and `Architecture`, which no refinement phase ever rewrites; the refinement writes the five after them. |
| `explore` | the repo map: `Ground` (free — glues `CLAUDE.md`, `docs/ARCHITECTURE.md`, `docs/PROJECT.md` and the tracked-file list verbatim) then one paid session that condenses it. A workflow wires both by implementing `Explored` on its state. |
| `sentinel` | the integration branch's own CI, read by `harness watch` on every full read, before the snapshot (`GitHub::branch_checks`): red opens one `harness:human` issue — found again by its title, never twice — and the router holds every task start and merge (`routing::base_red`) until the branch is green again, runs still in progress after a red included; green closes it. The reviews, repairs, rules, test debt and decisions go on. |
| `rule_waits` | (pure) what the rules' pending tests wait for, and which task unblocks them — for the router. `parse`: a `rules-status.json` text (the `rules-status` artifact `gates.yml` keeps on the integration branch) into `Waits`, each `pending` rule with the `kind:name` units its tests' `why` names (`waiting_for`: `waiting for a:b, c:d`; `no app: …` names none), each once; an unknown shape is no wait. `unblocked(task, waits)`: the rules one of whose missing units the task produces (`touched`: its unit is that kind — `screen` is a composition — and its `produces` names it, case aside) and those it produces every missing unit of (`completed`). `bonus`: 4 a rule touched, 8 more a rule completed, capped at `RULES_CAP` (100), +5 for a `DataCapability` — the router's `schedule` adds it to a `DevTask` under `routing::DEV_TASK_CEILING` (779, just under a review). It orders work, never holds any. |
| `branching` | a milestone's branch name, and the local half of its catch-up: `catch_up_locally`, the merge of the integration branch's head — the sha the watch read — in the harness's checkout through `Repo`, `Process` and `Disk`. Every conflicted file the table of generated files names (`harness_architecture::generated::GENERATED`: `Cargo.lock`, the registry's `Cargo.toml`, `queries/registry.json`, `mock/components/*.js`, and `mock/pages.json` when only its `builtFrom` digests differ) is taken from the base — removed (`Repo::remove`) when the base deleted it (`UD`, `DD`, `AU`: `checkout --theirs` refuses those) — then its generators run once each, in order (`scripts/register.sh`, `scripts/queries.sh`, `mock/build.sh`, `cargo metadata` — the cargo the merged tree pins, the launcher's `toolchain::by_the_tree`), and the merge is committed and pushed: `CaughtUp::Locally`. `BaseHead` carries the sha and the generators the merge stales (`catch_up::staled`): those run too, after a clean merge as well (`mock/build.sh` only where `mock/pages.json` records a `builtFrom`), what they rewrote committed on top of git's merge. A merge that meets the head's handler split — a legacy `mock/src/commands.js` / `reads.js` the branch changed and the head deleted (`UD`) — takes it first (`handler_split`, below) and merges again. A conflict anywhere else (a Rust source, a handler file both sides changed beyond a three-way merge) is `CaughtUp::Residual`, a failed step `CaughtUp::Failed`: both abort the merge, never hand-merge it. The whole merge runs on a detached `HEAD` (`checkout --force origin/<branch>`) and is pushed as the branch (`Repo::push_head`, `push origin HEAD:refs/heads/<branch>`): no local branch ever holds a commit not pushed yet, so a run killed midway (the view's hard stop) leaves nothing `branches_at_risk` reads as work kept at every later release. A push refused as a non-fast-forward (`[rejected]`: a session pushed to the branch meanwhile) is `CaughtUp::Deferred`, merged again next tick, never parked. Whatever is not `Locally`, the checkout goes back to the branch as the remote has it (`checkout --force`, `reset --hard origin/<branch>`, `clean`): a split or merge commit left unpushed would read as work kept in the checkout (`Event::WorkKept`, a false alarm), a detached merge base as a checkout astray. `pr_fix`'s conflict block reads the same table (`generated::instructions`), so the two never drift; it also carries `harness_architecture::mock_handlers::on_conflict` — the same method as `handler_split`, written for a session (one strategy, no `-s ours`) — and `mock_handlers::RULE`, which the code prompt states too. |
| `handler_split` | (crate-private) the catch-up across the legacy handler split, called from `branching`'s merge. In the one checkout, in sequence, never a worktree: abort the merge; split the merge base (`Repo::merge_base`) on a detached checkout — the head's `scripts/split-handlers.mjs` and `mock/src/server.js` written over the tree (`Repo::show` + `Disk`, never staged: the splitter refuses while `server.js` imports a legacy file), `node` run, everything committed there and left on no branch; check the branch's tip out again (detached, its sha read before), split it the same way and commit `mock_handlers::SPLIT_STAGED` only (`Repo::stage_paths`), the inputs put back (`reset --hard HEAD`, `clean`) so the merge brings the head's; merge again. Each handler file both sides added (`AA` under `mock/handlers/`) merges three ways on the split merge base (`Repo::merge_file`, the three sides written under `.git/harness-merge/`, out of `add -A`'s reach; ours `:2:`, theirs `:3:`, base `<split>:<path>` or empty): clean → written into the tree; conflicted → residual; an error → failed. The generated files then go through `branching`'s own loop, `mock/build.sh` always. The splitter refusing the branch → `Residual` on the legacy files, its reason said; refusing the merge base (or no merge base) → every add/add stays residual, the reason said in the journal; unable to start, killed, or refused for its node (`mock_handlers::SPLITTER_TOO_OLD`) → `Failed` (the toolchain's fault, parked, never a session). Covers both callers of the local merge: a milestone's catch-up and the merge train's refresh of a legacy task PR (`milestone_merge::Attempt::Stale`). A `DU` on a legacy file (the receiving side split, the incoming one not) stays residual. |
| `catch_up` | every building milestone branch kept up to the integration branch's head. `read_head`, every tick: the head (`branch_sha`) and, while a branch is behind it and it is not green (running, or red until a rerun), that commit's own checks (`GitHub::commit_checks` — never the branch's newest run, which a fresh push leaves to the head before); answers whether the board must be read in full (the head moved, its verdict came in, a catch-up due is unsettled). `read_head` also reads, on a move, the head's commit date (`GitHub::commit_time`, the reading's second when it cannot be read or lies ahead), and answers true too when a catch-up settled since the last full read (`CatchUps::reread`: its released tasks start the next tick, not after the quiet ticks), when a lag hold just ran out, or when a residual's retry or a parked failure's try is due (the head green). `CatchUps` (in the router's `Pulse`): the head, its verdict and its date (`seen_at`), what each branch was brought to, the residual conflicts (head, next retry, whether the no-landing warning was said) and the parked failures (`Parked`: the end of the park, the next recheck); `read` — at a full read, forgetting the marks of a branch that is no target or holds the head, the branches `due` (behind the head, once its own checks are green — never merely "not red" — a parked one once its recheck or its park's end is due, a residual once its retry is due) and `holding` (those whose tasks wait: a residual, a park still running, or **a branch lagging the head** — the head green and its catch-up not made, or its checks not concluded within `LAG_HOLD_SECONDS`, 20 min from the head's commit; #219 started on a branch one minute behind the milestone it needed). `lag_expiry` names, once per head, the head whose hold ran out with no verdict and the milestones it releases (a red head is a verdict: `base_red` holds its tasks, no expiry, no extra read); `routing::read_catch_ups` warns it. `catch_up_held` therefore means "waits", not "stuck": a lagging branch is in it from every move to its catch-up. `catch_up` merges that very sha: GitHub's merge, else the local merge (`LOCAL_PER_TICK`, one a tick) — straight to the local merge, its bundles rebuilt, when the merge stales the mock-up's bundles (`staled`, two compares at most through `GitHub::changed_files`: the head changed a file every bundle is built from since the branch's merge base, or the head brings bundles and the branch itself changed such a file; a read that fails or a list GitHub cut counts as such a change), since GitHub's clean merge would leave one side's components with digests of the other side's file; held → `refresh_task_prs` merges the branch into every open task PR into it (never a force; a PR a lane works on, or whose task's session still runs — `Busy`, from the lanes' numbers and the boards — skipped; a PR the merge stales, `staled` asked the same way, left on its old base — the merge train refreshes it rebuilt before it lands, `milestone_merge::Attempt::Stale`, no repair session) and says which; residual → `route_residual` labels the open landing PR `harness:pr-fix` (the repair cap applies), or says it and leaves it, never `harness:human`; then GitHub's merge alone is tried again every `RETRY_SECONDS` (five minutes) at that head — a hand or the repair may have merged the base, and that merge tests and acts at once (milestone/24 waited the old hour for nothing) — quiet while the landing PR has the repair, warned once per head without one; failed → parked `PARK_SECONDS` (an hour) before the next local try, said once, GitHub's merge alone tried every `RETRY_SECONDS` meanwhile unless taking the head stales the bundles (only the checkout rebuilds them). A lane on the landing PR pushes the next retry or recheck five minutes on (no full read every tick while the repair runs). A lane working on the landing PR defers the catch-up (read from the PR, its label being gone once the repair started). `told` is the outcome line the watch reports under `catch_up`, which the view's journal closes its row with. The time is handed in: pure but for the `GitHub` port. |
| `contest` | what a PR waits on, and a settled rules contest followed through — read from the board alone, so a restart reads the same waits back. `waiting`: a PR waits while an open `harness:rules` issue is titled `Rules contested: PR #<n>` (nothing written), while its latest contest is closed and no resumed marker follows it through (`PrWait::Settling` — the window before its base catches up, where a repair would be paid against the old test), or while an issue a decided `wait` named in a `<!-- harness-waits-on: #N -->` marker on the PR still blocks; `<!-- harness-resumed: #N -->` clears it. The router drops a waiting PR from the repairs, the merges and its task's decision; its review runs. `settled`: one page of closed `harness:rules` issues, then, for a contest of an open PR of the harness's cycle not marked resumed on it yet, whose base holds the settled rules (`changed_files(base, integration)` names neither `rules.json` nor `tests/rules/` — after the catch-up) — or a contest about a milestone branch (`Rules contested: … on milestone/<n>`) whose open branch still carries `CONTEST.md` written for that contest (every rule id the file names is in the contest's title or body) and holds the rules, no contest of the branch open — neither one about it nor one of a PR whose head it is. `resume`: `CONTEST.md` deleted through the contents API (`GitHub::delete_file`, a commit on GitHub, no checkout) on the PR's branch when its own diff added it — never a copy it took from its base — or on the milestone branch; with no lift of its own, a task PR's base merged into its head (`merge_branch`, GitHub's update-branch) so its checks run on the settled rules — a landing PR's head moves with its catch-up instead, and a base that does not merge is said in the comment; then the resumed marker on the PR, which counts its repairs anew (`review::repairs`), and `Resumed`. Idempotent: no file left, or a marker there. A settled contest resets the repairs, and so does a landed contract change (below). **A contract wait** (H2): a repair that stopped on a contract another milestone produces leaves `<!-- harness-waits-on-contract: #K -->` (`CONTRACT_WAIT`, `contract_wait_marker`) — `waiting` answers `PrWait::Contract` until a resumed marker for `#K` follows it, whatever `#K`'s state, so the window between the change closing and the PR's base taking it is still a wait. `contract_landed`: for each PR waiting on a contract, `#K` closed **and** the PR's base holds the manifest (`changed_files(base, integration)` names no manifest under `crates/<system>/…/<kebab(name)>/` — a change handed back changed none) → `ContractLanded`; `resume_contract` posts the resumed marker (the repairs counted anew) with the change's last comment quoted, so the next repair reads it on the PR, and poses `harness:pr-fix`. `waits_decided` counts the contract markers too: they count in `MAX_WAITS`. `held(gh, pr, comments)`: whether a PR waits as far as its own comments and the open contests say — what keeps `main_agent_merge` from escalating a landing PR past its repairs while it waits (that PR carries no label the router lists). |
| `close_merged` | a task whose PR was merged into its milestone outside the merge train — a human, an old session's own merge (#463, #464 held #465) — closes. `due(gh, board, merged, branch, running)`: the open `harness:agent` tasks of a board, no lane on them, with a merged PR proving them — its `Closes #N` line (`delivery::closes`), or a title naming `#N` (`delivery::title_names`, never `#2480` for #248) **confirmed by the PR's head being the task's declared branch** (one `pr` read): a title alone proves nothing; a task whose comments carry the merged note already is left (told once, never re-closed after a human reopens it). `close_delivered`: the PR's body given its `Closes #N` (`delivery::with_closes`) when it lacked it — so the landing's `every_task_merged` reads the same proof — then `close`, the tail the merge train shares (`review-pending` / `needs-decision` off, `delivery::merged_note`, close). Routed as the free `Route::CloseMerged`. |
| `contract_ask` | a contract change asked by a session that stopped on it — the dev loop's code session or a PR's repair, one path. `ask(gh, requester, (id, need), integration, log)`: `Asked::TasksOwn(why)` for an aggregate or a contract of the requester's own milestone (the caller answers; nothing opened); `Asked::Existing(k)` when an open `harness:contract-change` issue already asks the same id — matched on `Request::parse(body).id`, whoever asked it, so a task PR and its landing PR never make the manager evolve one contract twice; else `Asked::Opened(k)` (label created on a board lacking it, `ContractChangeAsked`). A task requester gets `## Decision` (`kind: contract`) and a blocker on `#k`; a milestone requester (its landing PR's repair) neither — its blockers are its split's. **A landed producer's contract is not the requester's own**: `contracts-untouched` refuses a consumer editing a landed manifest (#482); the manager opens the producer task under the requester's milestone. |
| `branch_sync` | (pure) whether a decision tells a session to sync a branch itself: `prescribed(text, integration, own)` — the first clause (line, sentence, `;` part) with a sync verb (merge, pull, rebase, cherry-pick, catch up, up to date) and the integration branch as a whole token (bare or `origin/…`), or a `push` to a `milestone/*` branch. The verbs are whole words in their imperative or present form (`merged`, `pull request`, `merge train`, a bare `is up to date` describe, they prescribe nothing). Never caught: the PR's **own base** merged into it (for a landing PR the integration branch is its base), a push whose target — the first `milestone/*` branch within a few words after `push` — is the PR's own head (its base pushed to is caught; its base named as a merge source before the push is no target), a verb negated **just before it** (`not`, `never`, `don't`… within three words) or whose subject is the harness (within two words). A negation or the harness elsewhere in the clause clears nothing (`merge origin/main_agent into milestone/24, no other change` is caught). `decide` refuses an `answer` or a `repair` that holds one (`decision::parse`). |
| `fake_github` | `#[cfg(test)]` — an in-memory `GitHub` shared by the workflows' tests. A fake adapter, not a mock. |
| `fake_git` | `#[cfg(test)]` — an in-memory `Repos`: every verb recorded, the ones named failing fail, a merge conflicting on the files a test names (`FakeRepos::conflicting`). |
| `fake_disk` | `#[cfg(test)]` — an in-memory `Disk`: what it holds reads back, what was written to it can be re-read. |
| `fake_locks` | `#[cfg(test)]` — `Grants`, a lock nobody else holds. It exists so mounting a table creates no `.lock-*` directory anywhere. |
| `fake_databases` | `#[cfg(test)]` — an in-memory `Databases` for `scratch_sweep`: lists what it holds, drops what is not busy, refuses what it was told to, can be down or lost before the drops, can stop a pass short as a deadline would. Here with the other fake adapters, where `CLAUDE.md` sends a test looking for one. |

## 12. A workflow is N rounds

The shape comes from core: `Workflow<S>` carries the precheck, the lock, the
`remaining` counter, the resume point and the summary, and calls `round(turn)`
once per turn. **A single run is a workflow whose `remaining` is 1** — there is
no "OneShot shape" and "loop shape", there is a number.

So every workflow has one round:

- it doesn't branch → core's `Round<S>`, mounted in `orchestration/round.rs`,
  with its `tolerance` if it has one;
- it branches → a hand-written type implementing `Executable`
  (`dev_loop::TaskRound`; the reason is in `docs/ROUND-DRAFT.md`, variant B).

Before that there were two shapes — a hand-written loop and a `OneShot` — which
is the same five concerns written twice, and core's `Round<S>` served nobody
because a stage's tolerance lived at workflow level. It now lives on the round,
that is, on whatever iterates the stages.

## 13. The workflows today

The skeleton is the same; what differs is justified by the nature of the
workflow, not by its history. Two tables, grouped by what a run's **subject**
is — an issue, or a pull request — because that is what decides the shape of
its precheck, its lock and its trigger.

**The four that work an issue:**

| | `dev_loop` | `refinement` | `planner` | `split` |
| --- | --- | --- | --- | --- |
| what it does | picks a task (or the one `--task` names, as a lane of a parallel watch), builds it in one session from the SPEC `split` wrote, observes delivery — a merged PR on the read side, an open reviewed PR marked `review-pending` on the write side, and on either side the open PR of a session killed waiting on its CI (`pr_stands`) | rewrites an issue body into its five refined sections, never `Scope`, `Architecture` or `Rules` — only when a human asks, `split` writing the SPEC otherwise | opens the milestones of a roadmap item from the design dossier, each placed in the architecture (`systems`, `concepts`, side) and `harness:ready` | opens the tasks of a milestone, each one unit of the architecture and a whole SPEC read off the design dossier, each agent task `harness:ready`, each layer chained on the one before and parallel inside |
| trigger | the `harness` binary, or the router | the `harness:refinement` label | `harness:ready` on a roadmap item with no milestone yet | `harness:ready` on a milestone |
| state | `Loop` (`Resumable` + `Scoped`) | `RefinementState` (`Explored`) | `PlannerState` (`Explored`) | `SplitState` |
| `remaining` | the `--rounds` budget | 1 | 1 | 1 |
| the round | `TaskRound`, hand-written | core's `Round<S>` | core's `Round<S>` | core's `Round<S>` |
| stages | `code` — one session plans, builds and tests the task | `router`, the phase sections (business: 3, technical: 2), `coherence`, `human-advice` (business) or `graph-advice` (technical), `publish` | the map, `context`, `plan`, `publish` | `context`, `slice`, `publish` |
| `orchestration/` | + `round.rs` | + `round.rs`, `prompts.rs` | + `round.rs` | + `round.rs` |
| `checks/` | + `preflight.rs` | — | + `gates.rs` | + `gates.rs` |
| lock | — | per issue | per roadmap item | per milestone |
| resume | `Checkpoint` | the counter in the issue comments | — | — |
| tolerance | — | — | one retry inside the `plan` action | one retry inside the `slice` action |

**The two that work a pull request:**

| | `pr_review` | `pr_fix` |
| --- | --- | --- |
| what it does | advisory second opinion on a PR, in fresh context | one repair attempt on a PR whose CI went red |
| trigger | the `harness:to-review` label | the `harness:pr-fix` label **and** a check concluded in failure |
| state | `ReviewState` | `FixState` |
| `remaining` | 1 | 1 |
| the round | core's `Round<S>` | core's `Round<S>` |
| stages | `inline`, `brief`, `publish` | `context`, `fix` |
| `orchestration/` | + `round.rs` | + `round.rs` |
| `checks/` | + `gates.rs` | + `gates.rs` |
| lock | per PR | per PR |
| resume | — | — |
| tolerance | `InlineMayFail` | — |

- `dev_loop` writes its round by hand because `pick` (choosing the task) and
  `delivered` (marking it) are **`Action`s, not `Stage`s**: forcing them into
  core's `Round<S>` table would subject them to the `--stages` and resume
  guards, which have no meaning for either. The others have nothing outside
  their table, so their `round.rs` mounts core's `Round<S>` and stops there.
- `dev_loop` has **preflight** gates (labels, milestone, skills, installed
  dependencies) because several opus sessions follow, and a `stat` costs less
  than a `/code`.
- `dev_loop`'s `code` body is `Scaffold → Narrow → Ask → Widen`, then its
  relay and `MarkDone`. `Scaffold` (`action::scaffold`, unpaid, off under
  `--no-scaffold`) writes the skeleton of a `DataCapability` or a Capability
  whose folder does not exist — `harness_architecture::skeleton`'s templates,
  **uncommitted**, through `Disk`; `register.sh`, `queries.sh` and `cargo
  check -p <unit> --all-targets` through `Process` — and takes it all back
  (the folder, the query file, the generated files as they were) when one
  step fails: a failed skeleton is a template to fix, the round goes on
  without it. It never overwrites: an existing folder or query file, the
  task's branch already made locally or on `origin`, a dirty checkout — each
  a skip, said. Before `Narrow`, so the closure follows the skeleton's
  `Cargo.toml` to its aggregate and the test kit. `Ask` tells the session to
  start from the files `Loop::skeleton` lists and never copy a neighbour, and
  `ArchitectureHolds` refuses a round that left a `SCAFFOLD:` hole in one of
  them (`checks::architecture::holes_left`), as a blocking review on its PR.
- `refinement` moves its seven templates into `prompts.rs`: same rule 5, just
  too much text to sit with the table.
- `refinement` and `planner` prepend the two shared `explore` entries to their
  sequence; `pr_review` tolerates a failed `inline` pass because the summary can
  still say something useful.
- `pr_review::data::notes` stays **in French**: it is published on a GitHub PR
  for a French-speaking human.
- `pr_fix` is the only workflow whose **first** stage writes: it removes its own
  trigger label before paying for anything, so a PR nobody can repair does not
  buy one session per poll. Its own `mod.rs` carries the reasoning. Which
  PRs it is due on is `pr_fix::data::candidates::due`, read by the router: a
  label and a red check or a branch that conflicts with its base, **within
  the repair cap either way** (H2: #423's landing PR was repaired 21 to 25
  times, the red branch uncapped) — and a reviewed PR whose review blocks,
  whose branch conflicts or whose check failed, within the cap. Its table is
  `context`, `fix`, then `after` (free): `AskForFix` catches the session's
  `AGENT_LOOP_STOP: contract <id> — <need>` (the prompt asks it for a
  contract another milestone produces) into `contract_stop` instead of
  halting; `RaiseContract` asks the change through `common::contract_ask`
  for the PR's owner — a landing PR's milestone, else the task its body
  closes (`delivery::closed_task`: `Closes`, `Fixes` or `Resolves #N`, the
  merge train's reading) — and posts the contract wait on the PR, or, for a contract the PR
  may evolve, the answer as a plain PR comment (no decided marker: the cap
  holds); both pose `harness:pr-fix` (a waiting PR is never repaired, and a
  landing PR keeps a label the router lists). **Within the waits cap**:
  a PR with `contest::MAX_WAITS` waits already (decided or contract) gets
  no new wait — a landed change resumes the PR and counts its repairs anew,
  so repair → stop → wait → resume had no bound of its own — its owner is
  escalated instead (`milestone_merge::escalate`, a human's at the cap).
  `PushWhatWasCommitted` (the `Git` port, `Repo::unpushed`): local commits
  pushed (`push_head`) only when the session's `AGENT_LOOP_OK` line says it
  pushed, never what a session that said "push nothing" or stopped left;
  what is not pushed to the PR is **set aside** on
  `harness/unpushed/<pr>-<sha>` and the checkout reset to `origin/<head>`
  (shas journaled first) — left in the lane, it made the next mount refuse
  its reset (#496: no marker, one commit ahead; `decide` halted on
  `router-lane-0` at 18:05); "repair pushed nothing" said when the head did
  not move.
- All of them are wired into the binary — `dev_loop` directly (`harness`) and every
  one of them through the router's dispatch (`harness watch`). `pr_review` was
  the last to get a launcher entry point: its documented trigger used to be a
  hook on `gh pr create`, which needs a public URL this harness does not have.

**The one that works a version of the data model:** `data_diagnosis` — a
version of `main` or `main_agent` whose two sides (`schema.sql`, `model.json`)
disagree, right after its snapshot. Trigger: the router, once the snapshot
reports a divergence; state `DiagnosisState`; `remaining` 1; core's
`Round<S>`; one stage, `diagnose`, an Opus session in the snapshot clone whose
answer is the `DataModelDiverged` event; no lock (two diagnoses of one
version say the same thing), no resume, no tolerance.

**The one that works a file's tests:** `test_debt` — the issue the CI's
mutants rotation opens against one source file (`Test debt: <file>`,
`scripts/test-debt.sh`), listing the mutants no test caught. Trigger:
`harness:test-debt` on that open issue, routed last, when nothing else
waits; state `DebtState`; `remaining` 1; core's `Round<S>`; stages
`context` (reads the file and the findings, then consumes the label — the
`pr_fix` shape, one label one attempt; it also picks this attempt's branch,
`test/debt-<n>-<k>`, past every one the remote holds — a dropped PR's
branch stays there, and a reused name would be refused at the push),
`cover` (a session that writes the tests, proves them with `cargo mutants
--file`, and **stops at the open PR** — it merges nothing, `gh pr merge`
being denied to sessions), gated by `NamesAFile`, and `land` (free:
`LabelThePr` labels the PR on that branch `harness:debt-pr`, the label
created on a board that lacks it); lock per issue; no resume, no
tolerance. The router lands the labelled PRs (`data::landing::ready`):
green, mergeable, into the integration branch and touching no forbidden
path (`.github/`, `rules.json`, `tests/rules/`, `scripts/`, `contracts/`,
any manifest) → `Route::MergeDebt` (`milestone_merge::run`, its issue
closed), at most one per `routing::DEBT_MERGE_EVERY` (3 h: each moves the
integration branch, and every building milestone takes it); a failing
check, a conflict or a forbidden path → `Route::DropDebtPr` (closed with
the reason, the issue left for the next rotation; no repair, no decision)
— a drop for failing checks is held while the integration branch is red
(`Landing::Drop::red`: a red base fails every debt PR's CI alike). It never changes the code under test and
never claims the mutants caught: the next rotation that judges the file
says so, and labels the issue again if not.

**The one that writes the rules first:** `rules` — the rules the human
validated in the factory's design, as `rules.json` (contract P) and the
tests that prove them, before any development. Opened by the factory at
Initialize (`Rules: write the rules and their tests`) and when a rule
changes (`Rules changed:`, the changed rules marked `conform`), and by the
CI when a development writes `CONTEST.md` (`Rules contested: PR #<n>`,
`scripts/rules-gate.mjs contest`). Trigger: `harness:rules` and
`harness:ready`, routed right after the planner; **while a rules' issue that
is not a contest is open, the router starts no development task**
(`routing::rules_first`). A contest holds only the PR that contests: it
waits (`common::contest`) — no repair, no decision on its task, no merge —
until the contest is closed, and the settled contest is then followed
through for free (`routing::Route::Resume`). A rules' task whose PR is judged
faithful and still checking (`milestone_merge::RulesPr::Checking`) is neither
tapped nor attempted again — for `routing::RULES_HELD_READS` (30) full reads
at most, then released with a warning (a PR with no check at all reads as
checking forever); faithful and red, or faithful and conflicting,
is attempted again — `routing::TAP_REOPENS` (2) times per watch process, the
count reset by a restart or a quota stop.
State `RulesState`; `remaining` 1; core's `Round<S>`; stages `context`
(reads the rules from the body's `json` block, or the contest, then
consumes `harness:ready` — an unreadable body is said on the issue and
handed to `harness:human`), `write` (a session writes the rules and their
tests, or settles the contest, and opens a `harness:rules` PR it never
merges) and `fidelity` (a fresh session judges each test against its
criterion only, merges when all are faithful, hands the issue to a human
otherwise), both gated by `AsksSomething`; lock per issue; no resume, no
tolerance. The body's format is `rules::data::request` — the view writes
it with `body`, the workflow reads it with `read`.

**The one that decides where a human was asked:** `decide` — any open
issue carrying `harness:needs-decision`: the business advice wanted a
decision, the arbiter could not settle one, a task's PR still does not merge
after `MAX_FIXES` repairs, or a milestone's landing PR does
(`milestone_merge::escalate`, one escalation for both: the owner is the
task, or the milestone; never while a lane repairs the PR, never over an
owner's answer to the review question; a landing PR red or conflicting past
its repairs alike; a `harness:pr-fix` PR past its cap that no merge path
covers — a task PR never put under review — through
`milestone_merge::exhausted_repairs`, on the task its body closes), a session stopped
on an ambiguity or the breaker refused (`dev_loop::action::stop`, which
writes `## Decision` with `kind: stop|breaker` and hands the third stop of a
task to a `harness:human` task instead), or a person refused a port. A stop
asking for a port ports.json has not (`AGENT_LOOP_STOP: port <id> — …`) is
never decided, with or without a human gate (#53): `stop` keeps it as
`kind: port` and opens a `harness:human`
"Approve the port …" task blocking the task, approved or refused from the
view's Ports pane; a refusal comes here to do without it. A stop on what
another issue has not landed is never decided either: when the reason names
an issue (`#20`), a milestone's branch (`milestone/20-…`), or an id or a
path the plan's graph attributes to a milestone (`crates/session/`,
`dev_loop::data::waits`), and that issue is open and not
`harness:waiting-merge` (`tasks::still_blocks`), `stop` writes a GitHub
blocker on it, keeps the stop as `kind: wait` (the round kept, so the third
stop still counts) and records `TaskWaits` — no label, no session, until the
producer lands; the task itself, its milestone and a roadmap item are never
waited for. A stop on a task the plan lacks is never decided nor a person's
either: `AGENT_LOOP_STOP: task <slice JSON>` carries one slice, `tasks
[<slice>, …]` several (H2-MT: the code prompts ask for every missing unit
in one stop; `missing::asked_all`, at most `MAX_SLICES` (8), one title
once), each in `split`'s own shape (`split::data::plan::TaskItem`), read by
`dev_loop::data::missing` and judged by split's `spec_gaps` plus a unit, a
system, a `produces` and a `<type>/<slug>` branch. `stop` opens it with
split's renderer (`split::action::publish::body_and_labels`) — `## Scope`
with its `branch:` and a line back to the stopped task, `## Architecture`
with `id:` (its first `produces`), `## Business Goal`, `## Acceptance
Criteria`, `harness:agent` + `harness:ready` + its side (write side with
`harness:data-layer`) — as a sub-issue of the task's milestone, or reuses the
open `harness:agent` sub-issue of that milestone with the same title — or
the same id while that one has not started (no `review-pending`, not
`waiting-merge`: two changes to one unit are two tasks, #448/#473) — then
parks the task on all of them at once (one blocker each, `kind: wait`,
`wait — until #a, #b land`, `TaskWaits` each), with or without a human
gate. No session writes the SPEC: the stop carries it.
The slices that are not whole, or a task with no milestone, open one
`harness:human` "Missing task for #N" listing them, blocking the task — or,
without a human gate, a decision asked only when nothing could be opened. Routed right after the rules' task,
before any work it unblocks; a task carrying the label is not runnable.
State `DecideState`; `remaining` 1; core's `Round<S>`; stages `context`
(reads the issue, its `## Decision`, its comments, the task's PR and review,
then takes the label — one label, one decision), `decide` (opus: `answer`,
`merge`, `repair` — the repairs counted anew from `review::DECIDED_MARKER` —
`wait` — `on: N`, a blocker on #N written once and a
`<!-- harness-waits-on: #N -->` marker on the PR, so the PR waits whatever
board the router reads (`common::contest::waiting`), no label calling
`decide` again, no blocker on a milestone — or `cut`; the scope,
`decision::Scope`: a PR into the integration branch that is not the rules'
task's — a landing PR — is never `merge`d nor `cut`, and an `answer` or a
`repair` that tells a session to merge, pull, rebase or catch up the
integration branch into a branch, or to push to a `milestone/*` branch, does
not parse (`common::branch_sync`; the prompt says the harness catches
branches up, `{integration}` from `Config::integration`) — one retry with
the reason, then nothing written), gated by `NeedsASession` (an owner's answer, `decided-by: owner`,
is never decided over) and `DecisionParses`, and `apply` (writes `## Decision`
with `decided-by: ai`, carries it out, labels `harness:ai-decided`, records
`DecisionTaken`); lock per issue. A decided `wait` is never a round of
`milestone_merge::escalate`: the question is asked again at the same round
once the wait settles; past `REVIEW_DECISIONS` repair rounds, or
`contest::MAX_WAITS` (2) waits decided on one PR, the task is a human's, and
the comment counts the repairs and waits from the PR's own markers and
quotes the last decision.

**The one that evolves a contract:** `contract_manager` — any open issue
carrying `harness:contract-change`, which `common::contract_ask` opens when
a code session (`dev_loop::action::stop`) or a PR's repair
(`pr_fix::action::publish::RaiseContract`) stops with `AGENT_LOOP_STOP:
contract <id> — <what it needs>`: a
consumer never edits a contract another milestone produces (gate
`contracts-untouched`, rule 17), and the request blocks its task. Routed
right after `decide`, before any split: a contract blocks work already cut.
State `ManagerState`; `remaining` 1; stages `context` (reads the request, the
plan's graph — the id's kind and version, its producer, its consumers — and
the milestone the producer's task goes under: the producer's while open,
else the requester's), `evolve` (opus: the whole manifest at version + 1,
the fixture that stands for it, a summary, the producer's task drafted:
title, brief, `task_goal`, `task_acceptance` — its `## Architecture` the
harness's to derive),
gated by `NeedsASession` (an id the graph does not produce is the
requester's own: told, closed, nothing paid) and `ChangeParses`, and `apply`
(`put_file` the manifest, the fixture and the graph — `version` up, state
`evolving` — on the integration branch; the producer's task under the
target milestone, written as split writes a slice (`data::producer::item`:
the unit from the graph's kind, the system the id's namespace, `produces`
the id, the session's goal and criteria or defaults from the manifest and
the fixture; `split::action::publish::body_and_labels` and `missing::with_id`)
— a whole SPEC, no `TaskHasASpec` halt (#434, #503), its tap open; every consumer and the requester told; the
request closed, which unblocks the task; `ContractEvolved`); lock per
request.

**The one that works a task's graph:** `arbiter` — a task whose technical
refinement found the design could not hold its `## Architecture` as `split`
cut it: an id it builds on that `needs` does not name, written as a question
under `## Decision` by the refinement's `graph-advice` stage. Trigger:
`harness:needs-ai-decision`, routed before the dev loop, since the task it
settles is held until it ran; state `ArbiterState`; `remaining` 1; core's
`Round<S>`; stages `context` (the milestone's tasks and the repository's
manifests, and what is settled mechanically: an id a sibling `produces` is
chained, one the repository holds needs no producer), `consume` (the label,
the `pr_fix` shape), `arbitrate` (one short session over the board's index —
one line per task, never a SPEC — naming which task produces each id left,
gated by `NeedsASession` before and `VerdictParses` after) and `apply` (the
task's `needs` and `blocked_by`, a producer's `produces` where the id was
nobody's, the verdict under `## Decision`, a comment); lock per task; no
resume, no tolerance. What no agent may decide — a `produces` to change, a
task to cut, a task that has started, a verdict under 4/5, a question
raised a second time — goes to a human under `harness:needs-decision` with
the reasoning, the mechanical chains kept. `common::decision` is the
question's shape, shared with the refinement that raises it.

Seven more modules sit beside these, not in the tables above: `init_repo`,
`milestone_merge`, `main_agent_merge`, `data_snapshot`, `data_model_draft`,
`plan_graph` and `scratch_sweep`, all the deterministic-command departure described in §1, not a workflow. Neither has a `trigger` beyond its own subcommand or
router route, no `state`, no `remaining`, no round, no lock, no resume, no
tolerance — every column above would read "n/a". `init_repo` creates the
`harness:*` labels and the integration branch, installs the architecture's
files in the repository through a throwaway clone (`docs/ARCHITECTURE.md`,
the rules block of `CLAUDE.md`, `contracts/`, the CI gates and the project's
`ci.yml` for its stack, the write side's server, registry and
`scripts/register.sh`, which it runs through the `Process` port to write the
registry's `Cargo.toml` when the product has none yet, and `mock/build.sh`
when a write stales every bundle the product built
(`harness_architecture::tree::stales_every_bundle`: the mock server, the
screen host, the build — the bundles and their `builtFrom` digests go in
the same commit, or the upgrade alone turns `pages-mocked--bundles` red on
every branch built on it) — one commit,
nothing already there overwritten without `--force`; an older marked block
is replaced in place, the `CLAUDE.md` rules block and the two documents
each against its own version, so the short rules every session pays for
are revised without rewriting a product's documents), runs a read-only
audit, and writes the link into `.env.local`. The two merges are the two
levels of one fold: `milestone_merge` merges a task's PR into its milestone
branch once the agent review has run and every check is green — the write
side's PR, which the session leaves open — and marks the task delivered,
its body given `Closes #<task>` before the merge when it lacked it
(`delivery::with_closes`), never on a base whose merge stales the PR's mock-up bundles
(`catch_up::staled`, its green checks ran on the base it last took):
`Attempt::Stale` names the base's commit and the rebuild, and the launcher
refreshes the PR in the harness's checkout (`branching::catch_up_locally`)
— it lands once green again; a PR already asked for a repair is
`Attempt::Repairing`, left to it;
`main_agent_merge` opens, then merges, the milestone branch's PR into the
integration branch once every task is merged into it and its CI is green and
GitHub says it is mergeable (`pr_mergeable`: green checks from an older base
say nothing of a conflict) — a conflicting PR is `Conflicting`, a merge GitHub
refuses is `MergeRefused`, both told as warnings and never a `Halt` that stops
the watch; a red one is asked a repair (`harness:pr-fix`) within the cap and,
past it, `Escalated` — a decision asked on the milestone
(`milestone_merge::escalate`; not while a lane repairs it, `Config::repairing`,
nor while it waits, `contest::held`) — and is the **merge train** when the plan has a graph (`data/merge_train`):
a consumer waits on the producers of what it reads (`WaitingOnProducers`,
whatever its number) — those the graph's edges name, and those of what any
of its tasks needs, closed tasks included (`merge_train::landing_waits`: the
edge the plan forgot, #28 on #24's `OralBrief`, holds the landing), earlier
in plan order only; a landing marks `built` in `plan/graph.json` every id
whose manifest its PR carried; the landing moves the integration branch's
head, and every other milestone still building takes it through the router's
catch-up route (`common::catch_up`, `Route::CatchUp`), not the landing.
`data_snapshot` dumps the data model of every branch whose head moved and
archives it on the target's `harness/data-model` branch; `data_model_draft`
asks one session for the first `model.json` of a branch that has none, from
its dump and its code, and proposes it as a pull request a human merges;
`plan_graph` writes the plan's graph (`plan/graph.json`, contract R) from a
roadmap item's milestones as the board holds them, against the design
dossier — what the planner writes as it opens milestones, for a plan made
before the graph existed. `scratch_sweep` is the smallest: no subcommand, no
route, no `action/` — the watch's loop calls it after every tick, whatever the
tick did, and it runs at most every ten minutes (`config::EVERY`); it lists the
databases the factory's role owns through core's `Databases` port and drops —
a plain `DROP DATABASE` each, 200 at most (`config::MOST`), on one connection
within a minute (`config::DROPS_WITHIN`), the rest the next sweep's — those
whose name carries a creation time
(`data::names`: `scratch_<epoch>_…`, the product's `platform-scratch-db`
guard's; `rules_runtime_<epoch>_<pid>`, `schema_dump_<epoch>_<pid>`, the
installed scripts'), idle, no template, not named by `DATABASE_URL` or
`DATABASE_URL_READONLY`, older than half an hour (`data::policy`, pure, the
time a value the launcher hands in). A sweep that cannot reach the server is
one warning per outage, never a failed tick. Every one of them is reached by
its own subcommand or by the router (`crates/launcher/ARCHITECTURE.md`).

## 14. Adding a workflow

In this order, because each step makes the next testable:

1. `data/state.rs` — the state and its bounds (`Resumable`, `Scoped`,
   `Explored`… according to what core demands);
2. `data/` — the business rules, tested without network;
3. `ports.rs` + `config.rs`, their `fake`s included;
4. `action/` and `checks/` — what writes, what judges, never both;
5. `orchestration/stages.rs` — the table, which names the stages;
6. `orchestration/round.rs` — the round: core's `Round<S>`, or a hand-written
   type if it branches;
7. `orchestration/workflow.rs` — the shape: `impl Workflow`, and the `remaining`
   this workflow carries;
8. `run.rs` — `Request` + `build()`;
9. the launcher — the concrete adapters, and those only.

Then `mod.rs`: a summary saying what the workflow does and pointing at the four
directories. No logic in a `mod.rs`.
