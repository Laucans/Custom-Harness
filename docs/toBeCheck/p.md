# P: milestone parallelism — starved milestones pull, screens freeze, rules weigh

Shipped 2026-10-25 (lane A, lot P), not yet seen on a live `watch`.
Design, critique and decision:
`harness-dnd2-run/night/p-design.md` (§ Decision, 2026-10-25 03:20 Paris:
the critique's simpler alternative; P3a and P5 dropped).

## Context

ForgeDesMondes, 2026-10-24:

- **11:05, cap deadlock.** #22, #28, #29 built (`MAX_CURRENT` = 3). #28's
  last task #219 was `blocked_by` #24, which the cap kept unsplit: nothing
  moved until #24 was split by hand.
- **The graph's missing edges.** #28 `depends_on [18, 26]` but #219 needs
  #24's `OralBrief`; #29 `depends_on [19, 20, 22, 26]` but #465 needs #27's
  `SuggestionPill`. The tasks started, stopped on "missing", were parked
  `blocked_by` the milestone (#219 ×2, #465 ×1), and nothing kept their
  milestone from landing first.
- **Screens held wholesale.** `runs_ahead`'s `proves_rules` held every
  composition naming rules while *any* producer built (#143), and a screen
  waited on every Micro-UI it lays out.
- **#157 held 17 rules** and waited behind lone Micro-UIs of its milestone.
- `compositions-resolve` was a no-op: every screen uses the keyed layout
  form (`[{"PlayerStage": {}}]`), which its jq did not read.

## What shipped

- **P2, ahead work per task** (`merge_train::unlanded_before`): the router,
  the boards and the lanes judge each task against every milestone before
  it in plan order that has not landed, not only the graph's edges
  (`tasks::runs_ahead` / `held_by`). The lanes say which producer holds
  which task: `watch: lanes -> #28's tasks … wait for them to land (#216 on
  #20)`.
- **P2, the landing** (`merge_train::landing_waits`): a milestone lands only
  once the producers of what any of its tasks needs (closed tasks included)
  have landed, earlier in plan order only — `milestones_ready_to_merge` in
  the router and `main_agent_merge::run` (`WaitingOnProducers`).
- **P3b, screens freeze now**: `proves_rules` is gone; a composition stands
  on the mock-up's fragment of every Micro-UI it lays out (a Micro-UI still
  waits on another unit's Micro-UI, a table, a Concept). A screen parked
  `blocked_by` a milestone (the arbiter's or a stop's wait) waives it when
  every id it needs of that milestone is one the mock-up stands in for
  (`tasks::screen_stands_on`, judged on the graph by the router; the lane's
  `--task` run trusts the router for a screen's milestone blockers). A
  blocker it needs nothing of, a table or a Concept still holds it.
- **P3d**: a screen's `## Rules` (split) and the code prompt say: freeze
  against the mock-up now, a pending rule is not the task's to wait for,
  `rules-hold` proves it once its units are built; red here → pass or
  contest. And, for every screen, rules or not: a Micro-UI another
  milestone builds is not missing — lay its id out in
  `apps/screens/<screen>.json`, the fragment stands in, never stop on it.
- **P3c, `compositions-resolve`** (asset `gates.yml`): reads the keyed
  layout form; on a pull request into `milestone/*`, a Micro-UI planned in
  `plan/graph.json` (`kind: micro-ui`) with a fragment in `mock/pages.json`
  passes ("lays out X, mocked"); strict on a PR into `main_agent` and on
  every push. Checked against `main_agent` `a55bd4d`: 14 layout ids, all
  with a `micro-ui.json`, strict exit 0.
- **P1, pulled split** (`routing::pulls` / `pulled`): a building milestone
  that is starved (no startable task, none on a lane) or held at its
  landing (every task delivered) pulls an unsplit producer — earlier in
  plan order, a `split_candidate` — past `MAX_CURRENT`, one a tick, only
  when the cap actually holds it back (`splits(.., None)` would not start
  it). Starved, only what holds a task nothing else holds (no sibling,
  person or own PR cycle): a need no mock-up stands in for, or a milestone
  blocker; a mocked need waits for the landing. Held at landing, any need
  or blocker of its tasks. A producer that cannot split yet is pulled
  through: the first unsplit, splittable milestone it waits on in the
  graph. Journal when the split starts on a lane:
  `watch: #24 split past the cap — #219 of #28 needs Brief (starved)`, or
  `… needs X through #P (held at landing)` (`Event::SplitPulled`). A pull
  nothing can serve: `would be split past the cap … (X, starved), but its
  own producers are not split and none can be yet`, at `say`, once per
  change. Under lot D's disk floor no lane starts, a pulled split's
  included.
- **P4, rules weighting** (`common::rule_waits`): the router reads
  `rules-status.json` (artifact `rules-status`, `gates.yml` on
  `main_agent`) once per integration head with a verdict; a task's weight
  gains 4 per rule touched, 8 more per rule completed, capped at 100, +5
  for a DataCapability, never past 779 (just under a review). `saw:` ends
  with `rules: #157 unblocks 17, …`; a task with no `harness:ready` or
  parked that holds ≥ 5 rules is said each tick.

## Before the restart

- [ ] List, read-only, the composition tasks with `## Rules` on triggered
      milestones not owned by a PR cycle: they become startable at the
      restart (expected: the ones `say_held` named on 10-24 with only
      Micro-UI needs).
- [ ] #219: `harness:ready` off on 10-24 with a background loop re-adding
      it — check the loop is gone, or put the label back by hand once
      milestone/28 holds #519.
- [ ] `gates.yml` reaches ForgeDesMondes through `harness init-repo`
      (`--dry-run` first: it skips a diverged file). A product PR, a GitHub
      write: the owner's word. Until it lands, `compositions-resolve` stays
      the old no-op there, which holds nothing.

## Live checks

- [ ] **#219 and #465 start without their producer.** #219 (`blocked_by`
      #24) and any screen parked on a milestone whose needs of it are only
      Micro-UIs, Capabilities or DataCapabilities start at the restart,
      #24 split or not: no `split past the cap` for them. Expected: their
      PR green on `compositions-resolve` ("lays out OralBrief, mocked").
      A screen that also needs a #24 table still waits.
- [ ] **A starved milestone pulls a split past the cap.** When a building
      milestone has no startable task and one is held only by an unsplit
      earlier producer (a table, a Concept, a nested Micro-UI, a milestone
      blocker), `watch.log` shows `split past the cap — #T of #M needs X
      (starved)` once, when the split starts (not each tick), and
      `dev_loop=[…]` may then hold more than 3 building milestones.
      Expected: 0 hand splits like 10-24 11:05; at most one pulled split
      per tick; never a `split past the cap` while fewer than 3 build.
- [ ] **A held landing pulls too.** A milestone whose every task is
      delivered and whose landing waits on an unsplit producer pulls it
      (`… (held at landing)` on the line): no milestone sits all-delivered
      behind an unsplit producer for more than one tick with a free lane.
- [ ] **Pulled through.** A producer that cannot split yet has its own
      unsplit producer pulled (`… needs X through #P`); a pull nothing can
      serve is said once (`would be split past the cap …, none can be
      yet`), not each tick.
- [ ] **Free holds instead of paid stops.** A task needing a table, a
      Concept or a non-screen Micro-UI of an earlier unlanded milestone the
      graph does not name is held by the lanes' line, not started; a screen
      laying out another milestone's unbuilt Micro-UI lays out its id
      instead of stopping: 0 `AGENT_LOOP_STOP … missing` on such tasks over
      a night (baseline 10-24: #219 ×2, #465 ×1).
- [ ] **No early landing.** No milestone's landing PR opens while a
      producer of one of its tasks' needs is unlanded (`WaitingOnProducers`
      names it); no strict `compositions-resolve` or `rules-hold` red on a
      landing PR for a not-yet-landed producer.
- [ ] **A screen starts while its producers build**, `## Rules` or a
      milestone blocker it only lays out of included. Its PR
      is green, `rules-hold--status` shows its rules `pending — waiting
      for …`; the rules run green on a later landing or push once their
      units are built. Watch for a red blamed on the wrong milestone (the
      critique's A2): a journey rule red only once its last unit lands.
- [ ] **`compositions-resolve`** passes a screen laying out a mocked
      Micro-UI on a task PR into `milestone/*` ("lays out X, mocked") and
      refuses the same screen on the landing PR into `main_agent` until
      the Micro-UI's `micro-ui.json` is there.
- [ ] **The rules' read is once per head.** `rules-status: read at <sha>`
      appears once per `main_agent` head (plus one after a restart);
      `gh api rate_limit` over a night is not above 10-24's. The `saw:`
      line names the top unblockers; a DataCapability unblocking many
      rules starts before a lone Micro-UI of the same milestone.
- [ ] **Numbers to compare with 10-24**: lanes busy per tick (expected
      higher: screens and pulled splits), hours a screen task waited
      (expected near 0 once its milestone is split), landings per day.

## Not in this lot

- The planner still writes no edge from a task's needs (`graph::place_reads`):
  the router reads them at routing time (decision Q5).
- No max-current tied to the lane count: reopen once lot D's floor is live.
- A rule pending at the last landing is reported by nobody yet.
