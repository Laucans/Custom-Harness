# Parallel milestones — contracts mocked, several current, the train, the manager

## Context

The owner's design (2026-10-17): make development parallelisable. Four steps
— the plan's graph (contract R, `plan-graph.md`), contracts mocked until
built, several current milestones with a merge train and critical-path-first
lanes, a contract manager. Steps 2–4 shipped on 2026-10-18, exercised by
their tests and gates only: no live watch has run them yet.

## Shipped

- **Step 2** (`4faec1a`): `Graph::mark_states` (planned / mocked / built /
  evolving), `plan_graph::Standing` read off the checkout, the mock server
  applies a planned DataCapability with no effect, the tree and the closure
  accept an id the graph produces, gate `contracts-untouched`
  (`scripts/contracts-gate.mjs`, rules block v4, rule 17).
- **Step 3**: `routing::splittable` waits on the producers' *split* when the
  graph knows the milestone (old rule otherwise); `splits` orders by the
  critical path and caps the ahead work at `MAX_CURRENT = 3` building
  milestones; the router reads one board per building milestone
  (`building_boards`) and the lanes serve them in that order; the dev loop
  reads the board its branch names (`board::read_on`); `main_agent_merge`
  waits on producers (`WaitingOnProducers`), marks built ids in the graph
  and catches every building branch up after a landing.
- **Step 4**: `AGENT_LOOP_STOP: contract <id> — <need>` opens a
  `harness:contract-change` issue blocking the task (label created on
  demand); the `contract_manager` workflow (route after `decide`) evolves
  the manifest, the fixture and the graph, opens the producer's task, tells
  the consumers, closes the request. The code prompts teach the stop.

## To verify, on ForgeDesMondes

- [x] The restarted watch reads `plan/graph.json` off `main_agent` (the
      `saw:` line names `dev_loop=[18]`), and #18's lanes keep running
      (2026-10-18 05:58Z, `2dde2cb`).
- [x] With #18 building, the milestones the graph knows whose producers
      are split or landed get split, critical path first, at most 3 building
      at once: lanes 1–3 split #19 (the path), #21 and #23 in one tick, #20
      waits on #19's split. Needed `2dde2cb` first: the planner's GitHub
      `blocked_by` chaining had held every milestone.
- [x] A lane on another milestone's branch picks that milestone's task
      (`board::read_on`): lane 4 took #118 on `milestone/23-…`, its log names
      milestone #23 (06:03Z).
- [x] **Ahead work is what the mock-up stands in for**: while a consumer's
      producers build, only its Micro-UIs and screens run
      (`tasks::mocked_only`, the lanes log `#23's crates (#118) wait on #18
      to land`); everything under `crates/` waits. #118 stopped on #18's
      missing migrations (06:04Z, $0.47), #216 on #20's missing
      `crates/session/` (12:40Z) before the rule: held at the next tick,
      taken once the producers are `waiting-merge`. The in-process dev loop
      (`--parallel` ≤ 1) does not hold them yet. (seen 2026-10-24 04:44 Paris: `#28's tasks reading tables, Concepts or Micro-UIs still building (#219) wait on #26 to land — ahead work is what the mock-up stands in for`; #214/#216 taken again once #20 landed)
- [x] **A Micro-UI does not wait on its own milestone's crates**
      (`tasks::blockers_pending`): every Micro-UI of #19–#28 was
      `blocked_by` its capabilities, so ahead work was empty (13:10Z, 2 of 5
      lanes busy). After the fix (13:13Z) the lanes took #62, #63, #138,
      #166 at once — then halted on the quota (6%). To verify: those
      Micro-UIs deliver green against the fixtures alone, their PRs pass
      `pages-mocked`. (seen 2026-10-18 16:11 Paris: PR #221 (#62
      CampaignOverview) merged, the first Micro-UI delivered ahead on the
      mock.) (A screen now freezes ahead of its producers: see `p.md`.)
- [ ] **`pages-mocked` checks the `builtFrom` digests** (template and
      ForgeDesMondes' `main_agent`, `ab64636`, 13:35Z): the next PR that
      changes a Micro-UI's code without `mock/build.sh` goes red on this
      step, and the landing waits for `pr_fix` instead of carrying a stale
      digest onto `main_agent`. No more `main_agent` → milestone 409 on
      `mock/pages.json`. The watch's start now says `upgrade: diverged …
      kept: .github/workflows/gates.yml, …` — the product's copy gets no
      template fix by itself. (contradicted 2026-10-24 04:44–08:55 Paris: pages-mocked did turn PRs red (3 checks_failed), but 17 catch-up warnings `cannot take main_agent: it conflicts on … mock/pages.json`; the regenerating catch-up handles it since 20:50 Paris)
- [x] **A Micro-UI reads any system's Capabilities** (`55fb573`, product
      gate `6dc5e62`, 15:56Z): the resumed #136, #166, #167, #193, #210,
      #218 pass `system-frontier` with their cross-system needs; PR #226
      (#166) goes green on it once it takes `main_agent`. (seen 2026-10-18 18:12 Paris: PR #226 (#166) merged with system-frontier green; #136, #166, #167, #193, #210, #218 all delivered by 2026-10-21 23:07 Paris)
- [x] **The gate follows a merged PR** (`ArchitectureHolds::follow_merged`)
      — seen 16:52Z on #136: `#238 is merged into \`milestone/23-…\` — the
      tree read follows its remote`, no "the checkout has none" (the halt
      left was the SPEC's missing `produces`, fixed by hand):
      a round that squash-merges its PR and changes nothing after is judged
      on the tree *with* the merge — the log says `… is merged into
      \`milestone/…\` — the tree read follows its remote` and no "the
      checkout has none" / "outside the closure" on the unit's own folder
      (#166 at 16:13Z, #136 at 16:25Z before the fix).
- [ ] **A plan with a read-ahead is refused** (`PlanParses`): the next
      planner run on a plan laid out of order halts with `the plan has a
      cycle — … reads \`…\`, which … produces later` and retries; the graph
      it writes has `reads_ahead` empty and no `depends_on` cycle.
- [ ] **A screen's milestone waits for its Micro-UIs**: the next
      `plan/graph.json` lists, for a milestone with `screens:`, the Micro-UIs
      its pages compose under `consumes`, and `depends_on` names their
      producers (#28 would have waited on #18 and #24).
- [ ] **The split refuses a misplaced slice**: a plan answering
      `system: dataguard` for `campaign.*`, a UI slice with no `produces`,
      or two slices on one Micro-UI is handed back to the session with the
      gap named, before any task is opened.
- [ ] **The contract manager's three answers**: a request that needs no
      change is `hand_back` (comment + close, no task, no graph write); a
      shared Capability that must behave differently is `duplicate` (new id
      in the requester's system, graph entry under the requester's
      milestone, producer task, requester's `needs:` switched); `evolve`
      only bumps. Never again a version that changes no field (#227).
- [ ] **The data model is laid before the mock-up** (creation): on the next
      factory creation, `drawn()` logs `a Claude Code session lays the data
      model …` before the reference page, the drawing and componentize
      prompts carry `The data model the proof of concept is built on`, the
      fixtures use uuid ids, and the Data step refines (not redrafts) it.
- [x] **The boards are read until the lanes are offered what they take**
      — seen 18:02Z on `b220548`: `dev_loop=[19, 20, 25, 26, 28, 21, 23]`
      (two before), each board saying what it holds; #199 and #211 had
      been taken and delivered at 17:43Z as soon as #26 got read; #125
      waits on #117 (`blocked_by` posted), not on #18. The lanes are empty
      now because every ahead-able task is delivered: what is held reads a
      table, a Concept or proves a rule — the only goulot left is #18.
- [x] **The train merges the first green PR, not the first PR**
      (`milestone_merge::ready_to_merge`) — seen 18:21Z, first tick on
      `75dda68`: `tick: MergeIntoMilestone { task: 225, pr: "233" }` although
      #51/PR #105 (red on R4) comes first by number. Left to see: `milestone/18`
      pushed into #105's branch and `rules-hold` green there; then #51 merges,
      #52 and #53 run, #18 lands.
- [x] **The Data layer laid whole parallelises** (owner, 18:40Z): with #243,
      #244, #245 runnable on #18 (distinct aggregates, no blocker), the next
      ticks take three at once (`takes #243`, `takes #244`, `takes #245` on
      one tick), then #246, then #247/#248/#249 together, then #250 — the
      measure the owner asked for: lanes busy per tick and data-layer PRs
      merged per hour against the baseline (17:00–18:59Z: 15 takes, 0
      data-layer PR). Two tasks on distinct aggregates never collide on a
      migration number (fixed in each SPEC). (seen 2026-10-18 20:38 Paris: one tick took #243, #244, #245, #247, #248 on five lanes; merges per hour not measured)
- [ ] **The split numbers the aggregates' migrations in FK order**: the
      next Data layer split writes `crates/data/migrations/NNNN_<agg>.sql`
      in each aggregate slice's `technical`, one number per slice, a table
      after every table it references — no two slices with the same NNNN.
- [x] **An architecture drift on an open PR is its blocking review and its repair**: the next
      `ArchitectureHolds` halt on a task whose PR is open posts a `🤖 Review notes — the
      architecture gate` comment on the PR (`VERDICT: blocking`), poses `harness:pr-fix` on it
      (once — not again while it carries it) and marks the task `harness:review-pending`; the
      train waits, `pr_fix` restores the file, the review then clears it. Expected on #244 /
      PR #252 (applied by hand at 19:3xZ, same format); no `/code` paid twice on a halted task. (seen 2026-10-22 02:26 Paris: #207: `PR #387 reviewed as blocking and harness:pr-fix asked; #207 marked harness:review-pending`)
- [ ] **The split says copy, never move, for an aggregate's tables**: the next Data layer split's
      slices read "copies its tables from mock/data/model.json into data/model.json"; no PR of
      the layer touches `mock/data/model.json` (#243–#250 reworded by hand at 19:25Z).
- [x] **The Data layer's schema is one slice, first, and the aggregates do not chain**: the next
      Data layer split opens one `infrastructure` task writing every migration from the design
      (`plan::is_schema`), then one task per aggregate `blocked_by` the schema task only (a need
      on another aggregate is rerouted, `publish::chained`); `spec_gaps` refuses two aggregates
      without a schema slice, an aggregate before it, or one naming a migration. On
      ForgeDesMondes: #255 (schema, blocked by #243/#244/#245) then #246–#250 side by side —
      expected five lanes busy at once when #255 closes. (seen 2026-10-19 00:16 Paris: after #255 (schema, taken 21:47Z) one tick took #246–#250 on five lanes; a new split's shape not seen)
- [x] **Reviews and repairs take the free lanes, and a green PR merges first**: with several
      `to-review` PRs, the ticks route one `PrReview` per tick onto distinct lanes (the PR a lane
      holds is not offered again, `not_running`), and `MergeIntoMilestone` comes before `PrFix`
      and `PrReview` in `decide`. Expected on ForgeDesMondes after the redeploy: #251 (clean, green
      since 19:50Z) merges on the first tick; #252/#254 reviews run side by side. (seen 2026-10-24 22:36 Paris: PrReview #527 and #528 on lanes 3 and 4 in one tick, beside #526 on lane 2; weights MergeIntoMilestone 1000+ > PrFix 800 > PrReview 780)
- [ ] **The planner refuses a plan whose Data layer is not first**
      (`data_layer_not_first`): the next roadmap item's plan, if it lays an
      aggregate in a feature milestone, halts with `the Data layer is split
      over N milestones` / `come(s) before the Data layer` and retries.
- [x] **Ahead work resumes as producers land**: when #18 lands, the tick
      takes #117/#119/#123 (#19's data layer) and #112/#116 (#21's) on the
      next read, with no restart. (seen 2026-10-20 03:33 Paris: #18 landed 01:31Z; at 01:33Z the same watch took #125, #156, #157, #175, #201 on five lanes, no restart. #156/#175/#201 stopped at once: their branches lacked #18's merge)
- [ ] A consumer's tasks build against a producer's mocked contract; the
      mock server answers `mocked-data-capability` for a planned write.
- [ ] A consumer ready to land while its producer still builds logs
      `WaitingOnProducers([…])` and waits; after the producer lands, its
      graph entry turns `built` (one `chore(plan): milestone #n built …`
      commit on `main_agent`) and every building branch takes the landing
      (`brought into #…` in the run record). (contradicted 2026-10-24 04:42 Paris: graph entries do turn built (`chore(plan): milestone #n built …` on main_agent), but every landing says `brought into no other branch`; the branches took the landing later through CatchUp, by hand after #20/#21/#25/#26; `WaitingOnProducers` never logged)
- [ ] A session stopping with `AGENT_LOOP_STOP: contract …` opens the
      request, the task waits on it (**seen 13:53Z**: #167 → #220, `blocked_by`
      added ✓; the manager then failed `ChangeParses` on a Micro-UI's
      manifest, which it cannot evolve yet — #220 settled by hand), the manager's run evolves the contract
      (one opus session, ~$1–2), the producer's task appears under the
      producer's milestone with `harness:ready`, the consumer's task runs
      again and reads the new version. (contradicted 2026-10-24 05:20 Paris: the manager evolved `session.listScenes` (#433) and `campaign.saveEntity` (#502), but both producer tasks (#434, #503) halted `TaskHasASpec`: Goal, Criteria and `produces:` written by hand. Both requests were opened by hand after a pr_fix contract stop)
- [ ] A violation older than the round — the stale AiSettings bundle on
      `main_agent` parked #60, #110, #111, #114, #115 — is said as a warning
      (`architecture: N violation(s) were there before this round`) and the
      round goes on; one the round causes still halts it.
- [x] GitHub reads per tick stay well under the rate limit with 5 lanes
      and several milestones building: `gh api rate_limit` over an hour of
      watch shows a comfortable `remaining` (12:30Z: exceeded, 8 boards read
      twice per tick before the fix). (seen 2026-10-25 01:40 Paris: last `API rate limit exceeded` 2026-10-20T05:38Z, none over 10-20..10-24 with 4–5 lanes; `gh api rate_limit` itself not sampled)
- [x] At most `MAX_CURRENT` (3) milestones build beside the critical path's
      next step: the `saw:` line's `dev_loop=[…]` has ≤ 4 entries once the
      current 8 have landed. (seen 2026-10-24: the last 400 `saw:` lines hold at most 3 `dev_loop` entries; the cap deadlocked #28 on unsplit #24 at 13:05 Paris)
- [ ] The next plan (a new roadmap item) lays the Data layer whole right
      after the skeleton: one milestone, one `data-layer` task per aggregate
      of the dossier, before any consumer; the graph names it the producer
      of every aggregate, so no milestone waits on another's tables.
- [ ] A contract the graph does not produce is handed back without a
      session (comment on the task, request closed).
- [ ] The view shows the `Contract manager` line and the
      `contract-manager/evolve` prompt.
