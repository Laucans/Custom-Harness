# W: one decide per question, never beside its own repair

## Context

ForgeDesMondes #542 / PR #547 (milestone #28), 2026-10-25
(`harness-dnd2-run/DECISIONS.md` 11:06, `night/priorities-H.md` entry W;
`watch.out` lines 22262-22279). PR #547 was still blocked after 2 repairs,
so a decision was asked and decide #1 started on lane 0 (key #542). decide
takes `harness:needs-decision` off before its session (`ConsumeRequest`).
The next tick saw the task blocked and not asked, so it asked again (round
+1, label back on). decide #1 then posted its "repair" marker (10:04:32) and
`harness:pr-fix`. The tick after routed decide #2 (#542) and the pr-fix
(#547) side by side. Two identical "repair" verdicts (10:04:32 and
10:06:45), the agent-fix in between (10:06:14). About $0.4 wasted, and the
decision round went up by one, so the task reached harness:human one round
early.

Shipped in 1a14f04 (`fix(w): no second decide while one runs or its repair does`):

- No escalation for a task or milestone a lane holds (the lanes' keys):
  `milestone_merge::ready_to_merge` (`held`, the task's number too),
  `milestone_merge::exhausted_repairs` (`held`, the task the PR closes),
  `main_agent_merge`'s `past_its_repairs` (the milestone's number in
  `Config::repairing`).
- Routing: `Job::holds`. A `Decide` holds the open PRs of its issue. A
  `PrFix` holds the task or milestone its PR delivers. `plan` skips a job
  when its key is taken (a lane, or any job picked this tick), or a hold is
  a session's (a lane's key, or a paid job picked this tick): a free job,
  like a red landing PR's `MergeMainAgent`, never holds back its repair.
  A decision and a repair of the same task never run side by side.
- Replay with the fakes:
  `milestone_merge::tests::the_duplicate_decide_of_542_is_one_decide`
  (one decide).

## To verify on a live watch

- [ ] When a task's PR reaches "still blocked after 2 repair(s)", the line
      `watch: PR #N of #T -> still blocked …` appears **once** per decision
      round. It never appears again while `lanes -> lane k decides #T` runs.
- [ ] After decide says `repair`, the next tick routes `pr-fixs #N` and
      **no** second `decides #T`. On the PR, one
      `harness-decided: repair` marker comes before the next `agent-fix`.
- [ ] No `decides #T` starts while a `pr-fixs #N` of its PR runs, and no
      `pr-fixs #N` while `decides #T` runs (watch.out `router lanes ->`
      lines).
- [ ] A landing PR past its repairs is not escalated again while
      `decides #<milestone>` runs.
- [ ] A red landing PR (milestone ready to merge) gets its `pr-fixs #N`
      routed: `merges main_agent #M` each tick does not hold it back.
- [ ] The decision round in `## Decision` goes up by one per real decide
      (round 1 then round 2, then harness:human). It is never inflated.
