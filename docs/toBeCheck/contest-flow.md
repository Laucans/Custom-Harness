# Contest flow: a PR waits on its contest, the harness lifts CONTEST.md

Shipped 2026-10-25 (lane A, contest-flow lot), not yet seen on a live
`watch`. Design, critique and decision:
`harness-dnd2-run/night/contest-flow-design.md` (§ Decision, § Addendum).

## Context

ForgeDesMondes, 2026-10-24: PR #516 (task #486) contested R10's test
(contest #517, rules PR #518). pr_fix paid 2 repairs on a PR red only from
its own contest; two decide "wait on #517" verdicts counted as two rounds,
so #486 went to `harness:human` saying the harness "decided 2 times to
repair"; #517 was attempted 3 times while #518 was faithful and waiting on
`rust`; development idled on "waits on the rules' task"; once #518 merged,
#516 stayed red on a CONTEST.md nobody removed. And milestone/28 kept R10's
CONTEST.md (contest #459, settled) since 288cb60: every PR into it failed
the contest gate and opened a duplicate contest (#532 from PR #531).

## What shipped

- `workflows::common::contest`: a PR waits while its `Rules contested:
  PR #n` issue is open, while its latest contest is closed and no resumed
  marker follows it (`PrWait::Settling`: the window until its base catches
  up — #516 was red 7 minutes there, 21:15→21:22, and would have paid a
  repair or a decide), or while an issue a decided `wait` named
  (`<!-- harness-waits-on: #N -->`, posted by decide on the PR) still
  blocks; `<!-- harness-resumed: #N -->` clears it. The router drops a
  waiting PR from pr_fix, the merge train and its task's decide; its review
  still runs. Journal: `watch: pr #N waits on contest #K`.
- A contest no longer holds development (`routing::rules_first`); the first
  rules' task and `Rules changed:` still do. An open contest naming no rule
  id is warned once a full read.
- `milestone_merge::escalate`: a decided `wait` is never a round; 2 waits
  decided on one PR (`contest::MAX_WAITS`) → `harness:human`; the human
  comment counts repairs and waits from the PR's markers and quotes the
  last verdict.
- `milestone_merge::RulesPr`: a faithful rules PR still checking is
  `rules_on_pr` — no tap, no re-attempt, for `RULES_HELD_READS` (30) full
  reads at most, then released with a warning; faithful + red or
  conflicting → attempted again.
- `Route::Resume` (free, weight 1005): a closed contest whose PR is open and
  not resumed, once the PR's base holds the settled rules (after its
  catch-up) → `CONTEST.md` deleted by a contents API commit on the PR branch
  when its own diff added it — with no lift of its own, the PR's base merged
  into its head (update-branch) so its checks run again; a landing PR waits
  on its catch-up's push — then the resumed marker (repairs counted anew)
  and `Resumed` in the journal. A closed `Rules contested: … on
  milestone/<n>` whose open branch still carries a CONTEST.md naming only
  that contest's rules, and holds the rules, no contest of a PR whose head
  is that branch open → CONTEST.md deleted on that branch.
- A PR still settling after `SETTLING_TELL` (60) full reads →
  `SettlingStalled` (warning, notification `settling:#N`, cleared by its
  `Resumed`).
- `GitHub::delete_file` (port), `GhCli` (contents API DELETE).
- `GhCli`: check rows deduped on (run id, job name), newest attempt kept;
  never across runs.
- Assets: `gates.yml` tells the rules PR by `startsWith(github.head_ref,
  'rules/')` (3 places, no payload labels); `rules-gate.mjs contest <pr>
  <base>` warns and passes on a CONTEST.md inherited from the base. Products
  take them at their next asset upgrade.

## To check on the next real contest

- [ ] Contesting PR (a #516-like one): 0 `pr-fix` rows in the ledger
      between the contest's opening and its closing — expected **0 repairs**
      while it waits. Journal: `watch: pr #N waits on contest #K` every full
      read.
- [ ] No `needs-decision` posed on its task while it waits; no
      `harness:human` from `escalate` for it.
- [ ] Rules' task: exactly **1** attempt (write + fidelity) per request
      while its PR is in CI; no `TapOpened` for it while it is checking
      (`saw: … rules_on_pr=[K]`).
- [ ] Development goes on during the contest: at least one `DevTask`
      dispatched on another task.
- [ ] Between the contest closing and the resume: `watch: pr #N waits on
      contest #K settled (its base takes the settled rules first)` each full
      read, **0** `pr-fix` rows and no `needs-decision` on its task.
- [ ] After the rules PR merges and the PR's milestone catches up: a
      `Resumed` event (`CONTEST.md lifted on <branch>; PR #N resumed`), a
      commit `chore: lift CONTEST.md — contest #K settled` on the PR branch
      and a new CI run on it — **within one tick** of the base holding the
      rules, no human action.
- [ ] A PR resumed with no CONTEST.md of its own (inherited, like #531, or
      already removed by a pr_fix): a merge commit of its base on its head
      and a new CI run; the comment says `<head> holds <base> now`. A
      conflicting base: the comment says its checks still reflect the old
      base, and a pr_fix follows.
- [ ] A catch-up held (`catch_up_held`) under a settling PR: after 60 full
      reads one `SettlingStalled` warning and a `settling:#N` notification,
      gone once it resumes.
- [ ] A fresh CONTEST.md written on a milestone branch with an older closed
      contest (#437-like) naming other rules: **not** lifted.
- [ ] That PR then merges green, or gets a pr_fix with repairs counted from
      0 (`<!-- harness-resumed: #K -->` on the PR).
- [ ] milestone/28's stale R10 CONTEST.md (contest #459): after its
      catch-up, a `Resumed` event `contest #459 settled -> CONTEST.md lifted
      on milestone/28-…` and the commit on that branch; then PRs into
      milestone/28 pass `rule-tests-untouched--contest` (once the product
      has the new `rules-gate.mjs`, they pass with a warning even before).
- [ ] A PR into a milestone branch that holds a CONTEST.md (new gate):
      `rule-tests-untouched--contest` passes with `::warning::CONTEST.md is
      inherited from origin/…`, no new `Rules contested` issue.
- [ ] A rules PR opened with `--label harness:rules`: its first
      `node-gates` run skips `rule-tests-untouched` (branch `rules/<n>`).
- [ ] A PR with a re-run job: `pr_checks_green` true once the newest attempt
      of each job is green (old FAILURE still in the rollup); a landing PR
      with a red `pull_request` run and a green `push` run stays not green.
- [ ] A decided `wait` on a task with a PR: a `harness-waits-on` marker on
      the PR, no second decide while #N is open, and the decide asked again
      at the **same round** when #N lands; a second wait → `harness:human`
      with "0 repair(s) and 2 wait(s)".
- [ ] Restart the watch mid-contest: the same waits are read back.
- [ ] At deploy: tasks left on `harness:human` by the old counting (a
      `verdict: wait` in their body) cleared by hand, logged in DECISIONS.md.
