# Y, Z, AA: settled waits resume, contract waits wait for the build, unit tasks name their id

## Context

ForgeDesMondes, 2026-10-25 (`harness-dnd2-run/night/priorities-H.md`
entries Y, Z, AA; `DECISIONS.md` journal 16:47–17:53).

- **Y**: PR #569 (#568) and PR #580 (#576) went `harness:human` on the
  2-waits cap: "decided 0–1 repair(s) and 2 wait(s) on it … it stops
  spending on it". But the issues their waits named (#567, #582) were being
  built. #567 merged at 14:49:01 and #569 was escalated at 14:50:20. Both
  were resumed by hand (decided-repair + `harness:pr-fix`).
- **Z**: PR #580 waited on contract change #581 (`campaign.listCampaigns`
  v2). The contract manager wrote the v2 manifest and opened producer #582.
  The watch logged "contract change #581 landed -> PR #580 resumed" at
  15:55, as soon as the base held the manifest. #582 had not landed, so
  `rules-hold` stayed red and the repairs burned.
- **AA**: #574 (split, evolving `campaign.exportCampaign`) and #507 had no
  `id`/`produces` under `## Architecture`. The narrowed checkout left out
  the unit's own folder, and the round halted `ArchitectureHolds` on its
  own files.

Shipped on branch `fix/y-z-aa` (not committed yet when this was written;
see `git log` for the commit):

- Landed means on the PR's base (`contest::landed_on_base`), for Y and Z
  alike:
  - Closed or `harness:waiting-merge`, never cut (`contest::was_cut`:
    closed as not planned, or by `decide`'s `cut`).
  - Under the PR's own milestone, that is enough.
  - Under another milestone (`Issue::milestone`, read from the native
    milestone's marker), that milestone must be closed or
    `waiting-merge`, and the PR's base must hold what the integration
    branch has.
  - A home milestone `blocked_by` the PR's own milestone is a cycle.
  - A task on no native milestone (none known): landed, and the PR's base
    caught up with the integration branch.
  - Limit: a contract wait has no bound beyond the cycle. A producer
    milestone stuck for another reason keeps the PR waiting.
- Y. `milestone_merge::escalate` reads the issues the decided waits named
  (`waits_settle`):
  - Work not on the base yet: it asks nothing (`Escalation::Waits`).
  - All landed on the base: it resumes the PR (`contest::resume_decided`:
    `harness:pr-fix` first, then a resumed marker per issue, the repairs
    counted anew). Every caller journals "decided wait on #N landed -> PR
    #P resumed".
  - Held by a person, cut, or a cycle: the PR goes to a human, and the
    comment says which.

  A PR goes to a human on the waits cap only once every wait has landed
  and the repairs after it still fail, and the comment says so.
  `contest::waiting` and `held` hold a PR while its waited issue still
  blocks (open, not `waiting-merge`) and no person holds it.
- Z. The contract manager writes `producer: #<task> on #<milestone>` in the
  request's header before closing it (`request::with_producer`).
  `contest::contract_landed` then waits for that task to land on the PR's
  base, with the unit's folder as what it touched.
  - Producer under a milestone that waits on the PR's own (a cycle): the
    manifest is read, as before.
  - Producer cut, or its milestone cut: the PR's task goes `harness:human`
    and is told once. The PR keeps waiting, so no repair is paid.
  - A request with no producer line still reads the manifest, as before.
  - `resume_contract` adds `harness:pr-fix` before it posts the marker.
- AA. `Unit::has_an_id` (all but infrastructure/tooling).
  - Split's `spec_gaps` refuses a unit slice with no `produces`, so the
    split is re-asked with the reason.
  - Split's publish writes `id:` under `unit:`.
  - The split prompt says that a slice evolving an existing unit names it
    in `produces`.
  - The dev loop's `TaskHasASpec` stops a unit task with no `produces`
    before any session, never in silence. If the task's own words name the
    id (its `id:` line, its aggregate, or the one `<system>.<name>` in its
    title), it writes `produces:`/`id:` into the body. Otherwise it posts
    the message once on the task (`<!-- harness-no-produces -->`).
  - The missing-task opener and the contract manager already wrote both.
    The opener's `gaps` refuses the same slice to the session.
  - Open unit tasks without `produces` on ForgeDesMondes, read on
    2026-10-25 before the restart: none (6 open `harness:agent` tasks).
- Replays with the fakes:
  - `milestone_merge::tests::the_replay_of_569_resumes_once_567_landed_never_a_human_on_the_waits_cap`
  - `…::the_replay_of_580_resumes_once_582_landed`
  - `…::the_replay_of_569_waits_while_567_is_merged_only_under_another_milestone`
  - `…::a_decided_wait_on_a_landed_milestone_resumes_and_is_not_held`
  - `…::a_wait_on_an_open_issue_asks_nothing_and_one_a_person_holds_is_a_human_s`
  - `…::a_wait_on_a_cut_issue_or_a_cycle_is_a_human_s_never_a_resume`
  - `common::contest::tests::the_replay_of_580_with_582_cut_hands_the_pr_to_a_human_never_resumes_it`
  - `common::contest::tests::landed_is_on_the_pr_s_base_never_merely_closed_elsewhere`
  - `common::contest::tests::the_replay_of_580_lands_once_582_landed_not_once_the_manifest_did`
  - `split::data::plan::tests::every_unit_an_id_names_says_what_it_produces_even_one_it_evolves`
  - `dev_loop::checks::gates::tests::a_unit_task_whose_words_name_its_id_gets_its_produces_written`
  - `dev_loop::checks::gates::tests::a_unit_task_that_produces_nothing_and_names_no_id_is_told_once_and_stops`

## To verify on a live watch

- [ ] Y. Next time `decide` answers `wait on #N` for a PR: while #N still
      blocks, `watch.out` shows `pr #P waits on #N (decided)` every tick.
      There is no `still blocked after 2 repair(s)` line for that PR.
- [ ] Y. Within one tick of #N landing on the PR's base (closed or
      `waiting-merge`; under another milestone, once that milestone landed
      and the base caught up), `watch.out` shows `decided wait on #N landed
      -> PR #P resumed`. The PR gets `harness:pr-fix` and a comment that
      starts with `<!-- harness-resumed: #N -->`, saying "What this PR
      waited on landed on its base (#N)". Then `pr-fixs #P` runs. The task
      gets no `harness:human` and no `harness:needs-decision`.
- [ ] Y. A wait on a task under another milestone that has not landed:
      no resume, no repair, until that milestone is `waiting-merge` and the
      PR's base caught up.
- [ ] Y. If those 2 repairs fail with 2 waits on the PR, the task goes
      `harness:human`, and the comment says "Every issue its waits named has
      landed, and the repairs since still fail". Count these in a day (0
      expected while waits keep settling).
- [ ] Y. A wait on an issue that carries `harness:human`, one cut, or one
      under a milestone blocked by the PR's own, hands the PR's task to a
      human at once ("which a person holds", "which was cut", "which waits
      on this PR's own milestone").
- [ ] Z. Next contract evolution: the closed `harness:contract-change` issue
      body has a `producer: #K on #M` line in its header.
- [ ] Z. The waiting PR stays `waits on contract change #R` until #K lands.
      There is no `contract change #R landed -> PR #P resumed` before #K
      merges. If #K's milestone differs from the PR's, the PR also waits
      until that milestone is `waiting-merge` (or closed) and the PR's base
      caught up.
- [ ] Z. After the resume, the PR's next `rules-hold` run is green on what
      #K built (no burnt repair on the contract).
- [ ] Z. A producer closed as not planned: `contract change #R closed but
      its producer #K was cut -> PR #P handed to a human`, once.
- [ ] AA. Next split: every task with a unit other than infrastructure or
      tooling has `id:` and `produces:` under `## Architecture`. Check with
      `gh issue view` on a few. A split reply missing them shows one retry
      in the split's run.log.
- [ ] AA. No `ArchitectureHolds` halt on a task's own unit folder in a day
      (#574 class).
- [ ] AA. A hand-opened unit task with no `produces`: its body gets
      `produces:` when its title names the id, else one comment carrying
      `<!-- harness-no-produces -->`. No session is paid either way.
