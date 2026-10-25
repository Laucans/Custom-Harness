# H2, the rest: repair caps, contract waits, no branch sync in decisions, merged tasks close

Built 2026-10-25 (lane A, H2-rest lot, after the contest flow `d0ca9ce` and
H1-B+ `b8339dc`), not yet committed, not yet seen on a live `watch`.
Design, critique and decision:
`harness-dnd2-run/night/h2-rest-design.md` (§ Decision, owner's delegate,
2026-10-25 03:20 Paris — it overrides the design where they differ).
Evidence: `harness-dnd2-run/DECISIONS.md` (2026-10-21 → 2026-10-25),
`night/priorities-H.md` (H2, MT, the 12:25 / 13:56 evidence, AUDIT 02:45).

## Context

ForgeDesMondes, 2026-10-20 → 2026-10-24:
- Landing PR #423 repaired 21 to 25 times (~$9): the red branch of the
  repair route had no cap, and `main_agent_merge` relabelled it every tick.
- pr_fix stopped on a contract three times (#156, #423, #482/#500) with
  "push nothing": a human opened each contract change by hand; #482 was
  told by the manager to evolve a landed contract itself and was refused.
- decide told sessions to merge `main_agent` into their branch (#486 18:04,
  #219 22:09) — the catch-up is the harness's.
- Tasks whose PR was merged outside the train stayed open (#165, #207,
  #362, #463, #464) and blocked their dependents (#465).
- test-debt PRs could never merge (sessions are denied `gh pr merge` since
  28a9439, the prompt still said it).
- The contract manager's producer task had no SPEC: `TaskHasASpec` halts
  (#434, #503), patched by hand.
- #486 found four missing units over four rounds, one stop each ($4.88).
- #496 (12:25): a repair merged the base, went green locally, pushed
  nothing.
- Audit 02:45: write-side data-layer sessions still polled `gh pr checks
  --watch` (#452, #207) — the create-test skill they run said so; on #157 an
  owner's answer was asked over 9 times in 1 h 40; `AGENT_LOOP_STOP: port`
  on #53 opened no "Approve the port" issue (no human gate).

## What shipped

- **Repair cap for every PR** (`pr_fix::data::candidates::due`): a
  `harness:pr-fix` PR red or conflicting is due only within `MAX_FIXES` (2);
  a decided repair (`DECIDED_MARKER`) or a resumed marker counts anew.
- **One escalation, task or milestone** (`milestone_merge::escalate(owner,
  …)`): `main_agent_merge` asks a red landing PR's repair within the cap,
  past it `MergeOutcome::Escalated` → `## Decision` (`kind: review`) +
  `harness:needs-decision` on the **milestone**; never while a lane repairs
  the PR (`Config::repairing` = the lanes' numbers; the train's
  `ready_to_merge` gets `running` too), never while it waits
  (`contest::held`), **never over an owner's answer to the review
  question**. decide on a milestone reads its landing PR (its branch);
  `decision::Scope::lands_on_integration` refuses `merge` and `cut` of a PR
  into the integration branch (the rules' task's PR excepted); a decided
  `wait` on a milestone writes the PR marker, no blocker on the milestone.
- **Contract stop in pr_fix**: the prompt asks `AGENT_LOOP_STOP: contract
  <id> — <need>`; `AskForFix` catches it, `RaiseContract` asks it through
  `common::contract_ask` (shared with the dev loop) for the landing PR's
  milestone or the task the PR closes. Opened, or an open request asking
  the same **id** found (whoever asked it) → `<!-- harness-waits-on-contract:
  #K -->` on the PR + `harness:pr-fix`; the PR waits (`PrWait::Contract`)
  until `#K` is closed **and** its base holds the manifest
  (`changed_files(base, integration)` names no manifest of the id), then
  `Route::ContractLanded` posts `<!-- harness-resumed: #K -->` with the
  change's last comment, its repairs counted anew (Q1), `harness:pr-fix`.
  A contract the PR may evolve (aggregate, own milestone's) → answered on
  the PR, **no decided marker**. Contract waits count in `MAX_WAITS`, and
  the cap holds: at 2 waits a new stop opens no request and posts no wait,
  the owner is escalated (`harness:human` at the cap). The task a PR closes
  is read by `delivery::closed_task` (`Closes`/`Fixes`/`Resolves #N`).
- **A landed producer's contract goes to the manager**, never "evolve it in
  this task" (the third branch of the old `contract_is_the_tasks_own` gone).
- **decide never prescribes a branch sync** (`common::branch_sync`,
  `decision::parse`): an `answer`/`repair` naming a merge, pull, rebase,
  cherry-pick or catch-up of the integration branch, or a push to a
  `milestone/*` branch, is refused once with the reason, then nothing is
  written; a PR's own base (merged in) and own head (pushed to) are
  allowed (landing repairs), a push to its own base is caught; a verb
  negated just before it (≤ 3 words) or whose subject is the harness (≤ 2
  words) is clean, a negation or the harness elsewhere clears nothing;
  past tenses, `pull request`, `merge train`, a bare `up to date` are no
  verb. The prompt says the
  harness catches branches up (`{integration}`). No session deny rules (Q5).
- **Merged tasks close**: `Route::CloseMerged` (free) — `close_merged::due`
  over each building milestone's merged PRs: `Closes #N`, or a title naming
  `#N` confirmed by the PR's head = the task's declared branch; the PR's
  `Closes` line written when missing. The merge train writes `Closes #N` on
  the PR's body before it merges (`milestone_merge::ensure_closes`).
- **test_debt lands through the harness**: one branch per attempt
  (`test/debt-<n>-<k>`), the session stops at the open PR, the workflow
  labels it `harness:debt-pr` (label created if missing; new in
  `labels::ALL`); the router merges the first green one
  (`Route::MergeDebt`, at most one per 3 h — `DEBT_MERGE_EVERY`) or drops a
  red / conflicting / out-of-bounds one (`Route::DropDebtPr`, closed with
  the reason, issue left open) — a red one held while the base is red; the
  3 h hold set only by a merge that merged.
- **Producer task as a slice** (`contract_manager::data::producer::item`):
  rendered by split's renderer — `## Business Goal`, `## Acceptance
  Criteria`, `## Architecture` with `produces:` and `id:`; the evolve prompt
  asks `task_goal` and `task_acceptance`; an old reply without them gets
  defaults (chosen over refusing it: an `evolve` override in the prompt book
  would otherwise halt every change).
- **MT**: `AGENT_LOOP_STOP: tasks [<slice>, …]` (≤ 8, one title once);
  every writable slice opened or found, the task blocked on all, one
  `## Decision` (`wait — until #a, #b land`); dedup by title, or by id while
  the existing task has not started (no `review-pending` / `waiting-merge`).
  The code prompts ask to find every missing unit before stopping.
- **pr_fix pushes what it committed** (`Git` port, `Repo::unpushed`): only
  when its `AGENT_LOOP_OK` line says `pushed` (the prompt asks
  `AGENT_LOOP_OK: pushed <sha>`); otherwise (no marker, a contract stop, a
  failed push) the commits are set aside on `harness/unpushed/<pr>-<sha>`
  and the checkout reset to `origin/<head>` — the lane is never left
  holding them (#496 shape); `repair pushed nothing` warned when the head
  did not move.
- **Past-cap PRs nobody escalated**: a `harness:pr-fix` PR red or
  conflicting past its cap, no `harness:to-review`, not a landing nor a
  rules' PR → `milestone_merge::exhausted_repairs` escalates the task its
  body closes (`RepairsExhausted`). A landing PR conflicting past its cap →
  `MergeOutcome::Escalated` like a red one.
- **View / notify**: the blueprint shows pr-fix's `after` and test-debt's
  `land` stages; `RepairsExhausted`'s text is neutral (a landing PR is never
  merged nor cut).
- **Audit**: the create-test skill ends at the open PR (no `gh pr checks`,
  no `gh pr merge`); a port stop opens "Approve the port …" with or without
  a human gate.

Gates at build: fmt, clippy `-D warnings`, `cargo test --workspace
--all-features` (2863 passed after the review fixes), machete,
`dead-code.sh`. No new port method.
Mutants: not run (once at the end of the plan).

## Deploy (before the restart)

- [ ] Open test-debt PRs on `test/debt-*` without the label: add
      `harness:debt-pr` by hand (`gh pr list --search head:test/debt-`), log
      it in DECISIONS.md. The label itself is created by `init-repo`'s
      upgrade at watch start, or by the first `LabelThePr`.
- [ ] `.llocal/prompts.json` overrides of `decide`, `fix`, `cover`, `evolve`
      or the code prompts hide the new texts: list them, re-apply each from
      the new default or drop it. The parse guards (branch sync, landing
      scope) hold either way; an old `evolve` gets the producer defaults.
- [ ] Open tasks with a merged PR: list them (`gh issue list -l
      harness:agent -s open` vs each `milestone/*`'s merged PRs) — the first
      full read closes them, one `#N delivered by PR #m, merged into … outside
      the merge train — closed` line each.
- [ ] Landing PRs with ≥ 2 `<!-- agent-fix -->` since their last decided
      marker: expect one `RepairsExhausted` + one decide each at their first
      red tick.

## To check on a live run

- [ ] Landing PR red: at most 2 pr-fix ledger rows on it, then
      `watch: PR #n of #m -> still blocked after 2 repair(s): the harness
      decides` and one `decide` (kind `review`, on the milestone). Its
      verdict is never `merge` nor `cut` (grep `lands a milestone`). A third
      round puts `harness:human` on the milestone. Expected: ≤ 2 repairs + ≤
      2 decides per landing (against 21–25 repairs for #423).
- [ ] While a lane repairs that landing PR (`pr_fix=[n]` / lane on #n), no
      `RepairsExhausted` for it.
- [ ] decide on a task whose owner answered under `## Decision` (`kind:
      review`, `decided-by: owner`): no `still blocked` line and no
      `needs-decision` posed again (against 9 decides in 1 h 40 on #157).
- [ ] pr_fix contract stop: `PR #p: stopped on <id> — contract change #k
      opened, the PR waits on it`; a `harness:contract-change` issue in the
      same tick; the PR carries `harness-waits-on-contract: #k`; each full
      read says `watch: pr #p waits on contract change #k`; no pr-fix ledger
      row on #p until it resumes.
- [ ] A second stop on the same id (another PR, or the landing PR): `already
      asks <id> — waiting on it`, no second request.
- [ ] Once #k closes and the PR's base took the manifest: `watch: contract
      change #k landed -> PR #p resumed, its repairs counted anew`, then
      exactly one repair. Time from close to resume ≈ the base's catch-up
      (≈ 10 min, the measured CI); if the catch-up is held the PR waits —
      no stall notice exists for contract waits yet (watch it by hand).
- [ ] A stop on a landed producer's Capability opens a contract change (no
      "evolve it in this task").
- [ ] decide: no `## Decision` verdict telling a session to merge, pull or
      rebase `main_agent`; when the first answer did, the run log shows the
      retry reason (grep `prescribes a branch sync`).
- [ ] A task PR merged by hand: its task closed within one full read
      (`outside the merge train — closed`), dependents runnable next tick.
- [ ] A harness merge of a PR whose body lacked `Closes #N`: `its body
      lacked Closes #N — written before the merge` in the journal; the
      landing later reads `every_task_merged` true (no `NotReady` loop).
- [ ] test_debt: the session's PR is on `test/debt-<n>-1` (or `-2`… after a
      drop), labelled `harness:debt-pr` by the workflow; merged once green
      (`MergeDebt`), the debt issue closed with the merged note; never two
      debt merges within 3 h; a red one closed with `Closed by the harness:
      its checks failed: …`, the issue left open.
- [ ] A new producer task (contract manager) runs its first round with no
      `TaskHasASpec` halt; its body shows `## Business Goal`, `## Acceptance
      Criteria`, `produces: <id>` and `id: <id>`; no hand-written SPEC.
- [ ] A code session missing several units: one stop `AGENT_LOOP_STOP:
      tasks [...]`, N issues opened, N blockers, one `## Decision` `wait —
      until #a, #b, … land`. Cost: one session against N (#486: 4 rounds,
      $4.88).
- [ ] Two different changes to one unit asked once the first is in review:
      two tasks, not one parked on the other.
- [ ] A third contract stop on one PR (2 waits already): no new
      `harness:contract-change`, `stopped on <id> past its 2 wait(s) — no new
      wait, #n escalated` in the run log, `harness:human` on the owner.
- [ ] A repair that answered without `AGENT_LOOP_OK` and committed: `set
      aside on harness/unpushed/<pr>-<sha>` in its run.log, the branch on the
      remote, and the lane's next mount resets cleanly (no unpushed-commits
      refusal on `router-lane-N`).
- [ ] A task PR labelled `harness:pr-fix` only, red past 2 repairs:
      `RepairsExhausted` once and one decide on its task.
- [ ] A landing PR conflicting past 2 repairs: `still blocked …` and one
      decide on the milestone (no `Conflicting` line every tick).
- [ ] main_agent red: no `DropDebtPr` with `its checks failed`; conflicts
      and forbidden paths still dropped.
- [ ] A decide verdict describing a merge (`was merged into main_agent`,
      `pull request into main_agent`) is not refused (grep `prescribes a
      branch sync` → none for it).
- [ ] A repair that committed without pushing and answered `AGENT_LOOP_OK:
      pushed …`: `pushed what the repair committed (k commit(s))` in its
      run.log and a new CI run on the PR; one that said it pushed nothing:
      `did not answer AGENT_LOOP_OK — not pushed` / `repair pushed nothing`.
- [ ] A write-side data-layer session's `session.log` shows no `gh pr checks`
      after `gh pr create` (create-test skill fixed).
- [ ] `AGENT_LOOP_STOP: port …` under `--no-human-gate`: an "Approve the port
      … for #N" `harness:human` issue (Ports pane), `HumanTaskOpened`, no
      decide paid.
