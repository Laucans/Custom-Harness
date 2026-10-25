# Every building milestone branch follows a green integration branch

**Status: on `main`, not committed yet, not seen live.** Replaces
`catch-up-held.md` (its "merge it by hand" line is gone).

## Context

ForgeDesMondes, 2026-10-24 (H1 round B + H4): after each milestone landed,
every building milestone branch conflicted on generated files
(`mock/components/*.js`, `mock/pages.json`'s `builtFrom`,
`queries/registry.json`, `Cargo.lock`) and a supervisor resolved them with
agents following `harness-dnd2-run/tools/catchup-recipe-m26.md`. The harness
caught branches up only after its own landing (GitHub's merge, a refusal
just commented) and before a task started (Cargo.lock + registry only). A
push to `main_agent` that moved no issue (a rules contest settled, the
plan's graph, an upgrade) reached no branch, and the open task PRs kept
their old base (`tools/catchup-prs.sh` by hand).

## What shipped (uncommitted on `main`, after `28a9439`)

- `harness-architecture::generated`: one table of generated files
  (`GENERATED`), their generators in run order (`register.sh`,
  `queries.sh`, `mock/build.sh`, `cargo metadata`), `digests_only` for
  `mock/pages.json`, `instructions()` for `pr_fix`'s conflict block.
- `branching::catch_up_locally` → `CaughtUp::{Locally, Residual, Failed,
  Deferred}`: the base's side of every generated file (removed when the
  base deleted it, `UD`/`DD`/`AU`), the manifest resolved when only digests
  differ, generators run with the merged tree's own pin
  (`toolchain::by_the_tree`: rustup's proxies ahead), commit + push; any
  other conflict is residual, aborted, never hand-merged; a push refused as
  a non-fast-forward is deferred to the next tick, not parked.
- `common::catch_up`: the head of `main_agent` read every tick (one
  `branch_sha` call) and, while a branch is behind it, **that sha's own**
  checks (`GitHub::commit_checks`, `gh run list --commit`, one more call
  until they conclude); `Route::CatchUp` (free, weight 1010) for every
  building branch behind a head whose own checks are green, merging that
  exact sha; GitHub first, at most one local catch-up a tick; failed →
  parked one hour, said once; residual → the landing PR labelled
  `harness:pr-fix` (cap applies) or said once, then GitHub's merge alone
  retried hourly at that head (said again when no landing PR); held →
  every open task PR into the branch takes it (`merge_branch`, never a
  force; PRs a lane works on, or whose task's session still runs,
  skipped), said by number. Each outcome is a `watch: catch_up -> …` line
  the view closes its journal row with (station `milestone-merge`).
- A merge that stales the mock-up's bundles (`catch_up::staled`: the head
  changed a file of `tree::stales_every_bundle` since the branch's merge
  base, or brings bundles while the branch changed such a file — one
  compare, a second only when the head brings bundles) skips GitHub's
  merge: the local catch-up runs `sh mock/build.sh` even without a
  conflict (H1 round D, `docs/toBeCheck/mock-handlers.md`); it counts in
  the one-local-a-tick budget. A task PR the merge stales is not refreshed
  by GitHub: the merge train refreshes it rebuilt before it lands
  (`milestone_merge::Attempt::Stale`).
- Holds: a branch behind or unable to take the base holds its tasks; a
  catch-up waits while a `pr_fix` lane works on its landing PR, that repair
  while the catch-up is due. `start_task` / `run_dev_loop` merge nothing
  themselves now; `main_agent_merge` no longer catches the others up.
- `pr_fix::data::candidates::due`: a `harness:pr-fix` PR that conflicts
  (checks green) is repaired within the cap, as a red one is.

## To verify live

- [x] After the next landing, the journal shows `tick: CatchUp { milestone:
      N, … }` for **every** building milestone within one tick of
      `main_agent`'s checks turning green **for the landed sha** — none
      before (right after the push, while GitHub has not created the new
      runs yet, nothing is due though the newest run is still the old green
      head). Quiet ticks stay quiet while the head is pending. (seen 2026-10-24 18:07–22:13 Paris: 4 heads — 22d39c4 green 18:06:29 → CatchUp 18:07:10, 19dfbe5 20:50:03 → 20:50:19, 19262b9 21:19:32 → 21:20:59, a3823e4 22:11:32 → 22:13:00; every building milestone listed, the `base=None` ticks before were quiet)
- [x] The merge commit on each milestone branch (GitHub's `Merge <sha>
      into milestone/N-…`, or the local `chore: merge main_agent (<sha>)
      …`) names the sha read, not a later tip. (seen 2026-10-24 22:14 Paris: the merges on milestone/24, 28, 29 name 22d39c4, 19dfbe5, 19262b9, a3823e4 — the head read each time)
- [x] A branch GitHub refuses is merged in the `catch-up` checkout:
      `milestone/N-… holds main_agent: mock/components/X.js and …
      regenerated, never merged by hand` — no agent, no supervisor; its CI
      (`gates`, incl. `pages-mocked--bundles`, and `rust`) is green on the
      pushed merge. At most one such line per tick; the others say
      `… its local catch-up waits for the next tick` and follow. (seen 2026-10-24 18:07 Paris: GitHub 409 on milestone/28 → `holds main_agent: mock/pages.json regenerated` (125e426, every check green); 20:50–20:54 one local catch-up a tick, the others `waits for the next tick`)
- [x] That checkout's `cargo metadata` ran with the pinned 1.99.1
      (`run.log`/watch toolchain line), and CI accepts the `Cargo.lock`. (seen 2026-10-24 20:39 Paris: the watch's toolchain line says `cargo 1.99.1 … pinned 1.99.1`; `rust` green on every pushed catch-up merge (125e426, 45cef01, ee39bcc, a185a47, 312a635))
- [x] A residual conflict (`mock/src/commands.js`, a Rust source) is said
      once per head: `… a conflict no script solves, on … — its landing PR
      #n is asked for a repair (harness:pr-fix)` then a `pr_fix` lane on
      #n; or `… has no landing PR open: left as it is …` once. No
      `harness:human` issue. Its tasks start no session meanwhile. (seen 2026-10-24 18:09 and 22:13 Paris: milestone/24 then /28 — `has no landing PR open: left as it is` once per head, held with no task session, no harness:human issue)
- [ ] Merge `main_agent` into such a branch by hand: within the hour (the
      retry), `milestone/N-… holds main_agent` and its tasks start again,
      `main_agent` unmoved. Without a landing PR, the hourly retry says
      `… still cannot take main_agent (…) … merge main_agent into it by
      hand` — once an hour, not once a tick. (not seen: milestone/24, merged by hand at 18:17 Paris, stayed in catch_up_held until the watch was restarted at 18:35 to clear it, before the hourly retry; H1-B refinement logged in night/priorities-H.md)
- [ ] A component deleted on `main_agent` while a branch rebuilt its
      `mock/components/X.js` (`UD`): the local catch-up removes it and
      `mock/build.sh` runs — no hourly `Failed` loop.
- [ ] A session's push landing on a milestone branch during its local
      catch-up: `… moved while it took main_agent — merged again at the
      next tick`, then held the tick after, no hour's park.
- [ ] No CatchUp of milestone N while a `route pr-fix <landing PR of N>`
      lane runs (watch.log `schedule:` lines), and no pr_fix of that PR in a
      tick where N's CatchUp is due.
- [ ] After a catch-up: `milestone/N-…: task PR(s) #a, #b hold it now` and
      those PRs' CI reruns; a PR on a lane is not touched, nor the PR of a
      task whose `harness --task` lane still runs. (partly seen 2026-10-24 19:15 Paris: `task PR(s) #500, #509 hold it now`, merge commits 6f5e24c/b317ec7, their full CI reran; the skip of a lane's PR or a running task's PR not exercised — no lane ran then)
- [ ] The view's journal shows a `catch up` row per catch-up, closed by its
      outcome (a residual or a failure in red), under `milestone-merge`.
- [ ] A failing catch-up (npx offline, a red generator) is one warning,
      `… tried again in an hour, its tasks wait`, not one per tick.
- [x] A push to `main_agent` that moves no issue (an upgrade, a rules
      contest settled) still reaches every building branch at the next green
      tick. (seen 2026-10-24 20:50 and 21:20 Paris: the upgrade 19dfbe5 and the settled contest 19262b9 reached milestones 24, 28, 29 at the first green tick)
- [x] After a `main_agent` push touching `mock/src/server.js` (or
      `screen.js`, `mock/build.mjs`, `mock/build.sh`): every building
      branch catches up locally, one a tick, each with a `chore: sh
      mock/build.sh after merging …` commit, and none through GitHub's
      merge; its `pages-mocked--bundles` green after. (seen 2026-10-24 20:50–20:54 Paris: 19dfbe5 touched mock/src/server.js; #28, #24, #29 caught up locally one a tick, none through GitHub — #29 with `chore: sh mock/build.sh after merging…`, #28/#24 with the rebuilt bundles in their conflict merge; node-gates (pages-mocked--bundles) green on all three)
- [ ] Cost: one compare call (`GitHub::changed_files`) per catch-up, two
      when the head brings bundles; one or two more per task PR refreshed
      and per merge attempt, on top of the reads below.
- [ ] Cost: one extra `branch_sha` call per tick, plus one `gh run list
      --commit` per tick only while a branch is behind a head that is not
      green (running, red, a `[skip ci]` or cancelled head: that one call
      every tick, never a full read); quiet ticks still read no snapshot.
- [ ] A red `main_agent` whose failed job is re-run green, same sha: the
      catch-ups follow at that tick, without a new push.
