# A code session ends at its pushed PR; a killed session with a PR delivered it; never two writers on one branch

## Context

ForgeDesMondes, 2026-10-18/19 (factory `~/.harness/factories/forgedesmondes`,
logs `.llocal/logs/agent-loop/<run>/{run.log,session.log}`, event log
`.llocal/harness.db`):

- Three write-side Data-layer sessions (#248, #249, #250, opus, $3.7–4.6
  each) pushed their PR (#264, #259, #260) and then waited on its CI as the
  write-side prompt told them to ("wait for its CI with `gh pr checks`"):
  `gh pr checks --watch` until Bash's 600 s cap, `sleep 30` loops, then a
  background poll "that will notify me" — and sat silent until the idle
  kill took them at 1200 s (the product CI's mutants step runs ~27 min).
  Each round was filed `FAILED`, the task never marked, and the scheduler
  reopened a code round on #248 while a `pr_fix` session was repairing PR
  #264 on the same branch: the second session stopped at once with
  `AGENT_LOOP_STOP: a concurrent pr_fix session (router-lane-2) is
  repairing PR #264 …`.
- 2026-10-19 20:45Z, the restarted watch's first tick took #248 and #250 as
  `DevTask` while PR #264 was under review on lane 0 and #260 open, and
  #246 while its green PR #262 was the same tick's `MergeIntoMilestone`.
- The read side was not failing: 225 read-side runs, 57 delivered, 0 killed
  as hung — its sessions wait on a short CI in the foreground and merge.

## What shipped

Commit `95dd36c` on `audit/code-ends-at-pr` (to merge locally into `main` once the shared checkout is free):

1. **Instructions.** The write-side prompt (`CODE_WRITE_SIDE`,
   `crates/workflows/src/dev_loop/orchestration/stages.rs`) says: open the
   PR with `--label harness:to-review`, say its number, END YOUR TURN; the
   harness watches its CI; never poll `gh pr checks`, never a background
   watcher, never Monitor, never a sleep loop or a scheduled wake-up; do
   NOT `gh pr merge`. The read-side prompt (`CODE`) keeps its own merge
   after a **foreground** `gh pr checks <n> --watch` (run again if it runs
   out), the same "never in the background" words, and says where a killed
   session's PR goes. `.claude/skills/code/SKILL.md` steps 5–7 say the
   same, per side. Tests on both prompts and on the skill text.
   `claude -p` now also runs with `--disallowedTools … ,Monitor`
   (`crates/adapters/src/agent/claude_cli.rs`, same mechanism as b23a77c).
2. **Dev loop.** `Halt::killed_as_hung()` / `KILLED_AS_HUNG`
   (`crates/core/src/domain/halt.rs`) is the one text the adapter's idle
   kill writes and the round reads. `TaskRound::perform`: a stage halt
   that is a hung kill goes to `MarkWaitingMerge::pr_stands` — the task's
   declared branch read, `gh.pr(<branch>)` (the port's `pr()` takes a head
   branch, as `gh pr view` does), and if a PR is open there: journal
   warning "the session was killed while waiting on PR #n's checks … the
   PR stands", `harness:to-review` posed on the PR when it lacks it (a
   read-side PR), the task commented and labelled `harness:review-pending`,
   `PrOpened` recorded, `Loop::pr_stands` set, and the round ends
   `Continue` — the postcondition `AMergedPrClosesTheTask` accepts
   `pr_stands` and `review-pending` on either side. No PR on the branch, or
   a closed one: the halt propagates as before (`FAILED`, exit 2).
   `note_review_pending` now marks either side (the read side's normal
   case never reaches it: its PR is merged); the label's description and
   `tasks::why_not` / `idle_reason` no longer say "write side".
3. **Router.** `Snapshot::task_prs` (`crates/workflows/src/common/routing.rs`):
   every open PR carrying `harness:to-review` or `harness:pr-fix` as read,
   on a lane or not, worth reviewing or not (`router::task_prs`, the union
   of the two labelled reads already paid). `routing::pr_cycle_owns(task)`:
   true when such an open PR names the task (`tasks::pr_names_task`: head
   == the body's `branch:` line, or the title carries `#<n>` as a whole
   token) or `tasks_to_merge` lists it; `routing::startable` refuses such a
   task, and the lanes' "what starts now" line reads the same function.
   Tests: under review by branch, under repair by title, a tick from its
   merge → not scheduled; PR merged (task closed by the train) → nothing of
   it; PR closed unmerged → startable again.

## To check on a live run

- [ ] A write-side session ends right after `gh pr create`: its
      `session.log` shows no `gh pr checks`, no `sleep`, no background task
      after the PR line; the run's `run.log` ends on `[code] AGENT_LOOP_OK`
      (or no marker) then `#n: PR #p is open, merged once reviewed and green
      — marked harness:review-pending`, under ~15 min for a data-layer
      task instead of 1h+ ending in `killed as hung`. (contradicted 2026-10-24 ~11:11 Paris: write-side #452 (data layer, 20261024-084334-452) ran `timeout 3000 gh pr checks 487 --watch` after its PR; #207 on 22 Oct `sleep 45; gh pr checks 387 --watch`; most write-side rounds do end at the PR (#520: 21 min), data-layer ones take 25–160 min)
- [x] `harness.db` / `watch.log`: no `printed nothing for 1200s and was
      killed as hung` on a round whose PR was already open; if one occurs
      (a session that ignored the words), `run.log` carries `warning: #n:
      the session was killed while waiting on PR #p's checks … the PR
      stands`, the PR carries `harness:to-review`, the task
      `harness:review-pending`, the lane exits 0 (not 2), and the next tick
      shows `pr_review=["p"]` with no `DevTask { task: n }` in `schedule:`. (seen 2026-10-25 01:40 Paris: since the merge (20 Oct 01:52) 109 rounds ended `marked harness:review-pending`, no dev-loop `killed as hung` — the 3 hung kills were pr_fix, the last 20 Oct 16:52; the fallback path not exercised)
- [x] While PR #p of task #n is open and labelled (`saw: … pr_review=[…]`
      or `pr_fix=[…]` or `to_merge=[n]`), no tick's `schedule:` line lists
      `DevTask { milestone: m, task: n }`, lane or no lane on the PR — in
      particular during a `pr_fix` on a lane (its label consumed, the PR
      still `to-review`). (seen 2026-10-25 01:40 Paris: 82 write-side PRs opened 20–24 Oct — no `schedule:` line offered `DevTask` for their task between PR open and merge or close)
- [x] The train still merges: `tick: MergeIntoMilestone { task: n, pr: "p" }`
      → `#n delivered` and the task closed; the next tick no longer holds
      it (closed, not on the board as open). (seen 2026-10-24 20:44 Paris: `MergeIntoMilestone { task: 514, pr: "515" }` → `#514 delivered by PR #515 … — closed`; 102 such ticks)
- [ ] A PR closed without merging (by hand): the next full read offers
      `DevTask { task: n }` again.
- [x] A read-side session (short CI) still merges itself: `gh pr checks <n>
      --watch` once in the foreground, `gh pr merge --squash`, `#n delivered
      by PR #p … — closed` — no regression in the 225-run read-side flow. (seen 2026-10-24 13:18 Paris: read-side #484 — foreground `gh pr checks 497 --watch`, `gh pr merge 497 --squash`, `#484 delivered by PR #497 — closed`; since d0a2a93 (24 Oct 15:52) the harness merges read-side PRs instead)
- [x] `claude -p … --disallowedTools ScheduleWakeup,…,Monitor` accepted by
      the installed CLI (no `! unknown tool` line in `session.log`). (seen 2026-10-24 22:15 Paris: 0 `unknown tool` in the session logs of 20–24 Oct; #520's session init lists no Monitor)
