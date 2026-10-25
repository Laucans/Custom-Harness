# A task that cannot start waits for free, through a GitHub blocker

## Context

ForgeDesMondes, 2026-10-18 (factory `~/.harness/factories/forgedesmondes`,
event log `.llocal/harness.db`): tasks #214 and #215 of milestone #28 went
through 23 code rounds each ($6.23 and $5.80) and 23 decide sessions each
(opus, $0.29 each). Every round the session stopped at once with
`AGENT_LOOP_STOP` on a producer that had not landed (#20's `crates/session/`
missing from `origin/main_agent`), the dev loop asked a decision, `decide`
answered "keep waiting", the body changed, the lanes' parking fingerprint
moved, and the task was taken again next tick. 60 of the day's 82
decisions were such "wait" answers.

## What shipped

Four commits, this file's own the last:

- `6982cf2` **`decide` has a fifth action, `wait`** with `on: N`
  (`crates/workflows/src/decide/`): the prompt offers it, parsing accepts
  `20`, `"20"` or `"#20"` and refuses a wait without an issue or on the
  issue itself, `carry_out` writes the blocker once (`blocked_by` read
  first), the comment says "Blocked by #N until it lands", `DecisionTaken`
  records action `wait`; `harness:needs-decision` is taken by
  `ConsumeRequest` as before and no label calls `decide` again.
- `a53ca9c` **The dev loop's stop handler parks at once, paying
  nothing** (`crates/workflows/src/dev_loop/action/stop.rs`): when the
  stop's reason names an issue (`#20`), a milestone branch
  (`milestone/20-sessions`), an id of the plan's graph (`Session`,
  `session.listScenes`) or a path the graph attributes to a milestone
  (`crates/session/` through the `session` system;
  `crates/data/migrations/0004_entity.sql` through the `Entity` aggregate,
  the id's producer before the system's milestone) —
  `crates/workflows/src/dev_loop/data/waits.rs`, pure — and that issue has
  not landed (`tasks::still_blocks`: open and not `harness:waiting-merge`),
  it writes `blocked_by(task, producer)` (not twice), keeps the stop under
  `## Decision` as `kind: wait` / `decided-by: harness` / `verdict: wait —
  until #N lands`, and records `TaskWaits` (journal line `#N parked until
  #M lands (wait): …`, silent for `notify`). The task's own number and
  milestone, a roadmap item (it closes after the task's own milestone:
  waiting on it would never end), a landed or unknown issue, name nothing:
  the needs-decision path is unchanged for them, as are the port, contract
  and third-stop rules — the wait keeps the stop count (`round`) it found,
  so a third stop after a wait still goes to a human. The graph is read
  from `plan/graph.json` on the integration branch (`RaiseDecision`
  carries `integration_branch`). The lane ends 0 as after any hand-over;
  the blocker, not the lane's parking, is what holds the task.
- `aa4a46c` **A milestone blocker is pending until it lands** —
  closed, or open with `harness:waiting-merge` (`main_agent_merge` labels
  it right after its PR merges): `blockers_pending` already read it so; it
  now goes through `tasks::still_blocks`, and the router's parking
  fingerprint (`crates/launcher/src/router/lanes.rs`, `issue_version`)
  counts the blockers still holding the task, so a parked task moves at
  the `waiting-merge`, not only at the close.

## To check on a real run

- [x] A session stopping with `AGENT_LOOP_STOP: … #20 …` (or naming
      `crates/<system>/` of a milestone still building): the task gets
      `blocked_by #20` on GitHub, its `## Decision` reads `kind: wait`,
      `watch.log` shows `#t parked until #20 lands (wait): …`, **no**
      `harness:needs-decision`, and the lane ends 0. (seen 2026-10-20 11:14 Paris: #214 stop `missing crates/session/read/Cargo.toml` -> `#214 parked until #20 lands (wait)` (in the run's run.log, not watch.log), blocked_by #20 on GitHub, no needs-decision, lane 2 exit 0)
- [x] The next ticks: the task is listed as blocked, no code session and no
      decide session is paid for it. Expected numbers for a #214-like task:
      1 code session (the stop) and 0 decide sessions until #20 lands,
      against 23 + 23 before. (seen 2026-10-24 04:51 Paris: #214 had 1 code session (0.33 $, 10-20 09:14Z) and 0 decide sessions until #20 landed (10-24 02:42Z); retaken 02:51Z)
- [ ] #20 gets `harness:waiting-merge` (its PR merged into `main_agent`):
      within two ticks the task is startable again, its milestone branch
      takes `main_agent` first (`catch_up`), and the session finds
      `crates/session/`. (contradicted 2026-10-24 04:46 Paris: #20 landed 02:42Z, but milestone/28 could not take main_agent (conflict on mock bundles): `#214 held … merge it by hand`; caught up by hand, then #214 delivered PR #432 at 03:13Z. Again 2026-10-24 22:07 Paris: #219 started before milestone/28 took #24's landing and stopped on the missing OralBrief/ToCreateList)
- [x] A stop naming something the harness cannot attribute (no `#N`, no
      branch, no graph id or system path): the needs-decision path as before.
      `decide`, offered `wait`, answers `{"action":"wait","on":N}` when the
      case is a producer not landed: the blocker appears, the comment says
      "Blocked by #N until it lands", `harness:ai-decided` is set,
      `needs-decision` taken, and the task is not taken again before #N lands. (seen 2026-10-24 20:19 Paris: #486 stop -> decide `wait` on #514; blocked_by #514 added, needs-decision removed, not taken again until #514 landed (18:44Z -> 18:45Z). The comment reads `Decision: wait on #514`, not `Blocked by #N until it lands`; ai-decided not checked)
- [x] A day's decisions (`DecisionTaken` rows in `.llocal/harness.db`): the
      share of "keep waiting" answers drops from 60/82 to about zero; the
      ones left should be `wait` actions on cases the stop handler could not
      attribute. (seen 2026-10-25 01:40 Paris: decision_taken 10-20..10-24: 45 decisions, no `keep waiting` answer left, 9 `wait` actions)
- [ ] The cost of a reason the attribution reads wrong: a `#N` that is an
      open, unrelated issue (a human task mentioned in passing) would block
      the task on it. Watch for a `TaskWaits` on an issue that is not a
      producer of the task's milestone; if it happens, narrow `waits::named`
      to milestones and agent tasks.
- [ ] A `decide` `wait` on an issue already landed (closed, or
      `waiting-merge`) holds nothing: the task runs again next tick and the
      old loop resumes for one round. Not seen yet; if it is, refuse it in
      `carry_out` (read the issue, treat it as an `answer`).
