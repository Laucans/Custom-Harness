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
- [ ] **Ahead work is read-side**: a consumer's `harness:data-layer` tasks
      wait until its producers land (`tasks::read_side_only`, the lanes log
      `#23's data layer (#118) waits on #18 to land`); its read-side tasks
      run against the mocks. #118 stopped on #18's missing migrations before
      the rule (06:04Z, $0.47): the next tick must hold it, and take it once
      #18 is `waiting-merge`. The in-process dev loop (`--parallel` ≤ 1) does
      not hold it yet.
- [ ] A consumer's tasks build against a producer's mocked contract; the
      mock server answers `mocked-data-capability` for a planned write.
- [ ] A consumer ready to land while its producer still builds logs
      `WaitingOnProducers([…])` and waits; after the producer lands, its
      graph entry turns `built` (one `chore(plan): milestone #n built …`
      commit on `main_agent`) and every building branch takes the landing
      (`brought into #…` in the run record).
- [ ] A session stopping with `AGENT_LOOP_STOP: contract …` opens the
      request, the task waits on it, the manager's run evolves the contract
      (one opus session, ~$1–2), the producer's task appears under the
      producer's milestone with `harness:ready`, the consumer's task runs
      again and reads the new version.
- [ ] A violation older than the round — the stale AiSettings bundle on
      `main_agent` parked #60, #110, #111, #114, #115 — is said as a warning
      (`architecture: N violation(s) were there before this round`) and the
      round goes on; one the round causes still halts it.
- [ ] A contract the graph does not produce is handed back without a
      session (comment on the task, request closed).
- [ ] The view shows the `Contract manager` line and the
      `contract-manager/evolve` prompt.
