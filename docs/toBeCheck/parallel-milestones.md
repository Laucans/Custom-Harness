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

- [ ] The restarted watch reads `plan/graph.json` off `main_agent` (the
      `saw:` line names `dev_loop=[18, …]`), and #18's lanes keep running.
- [ ] With #18 building, the milestones the graph knows whose producers
      are split or landed get split, critical path first, at most 3 building
      at once (`splits`); `watch: router lanes` names them in that order.
- [ ] A lane on `milestone/20-…` picks a task of #20, never of #18
      (`board::read_on`): the `--task` run's log names milestone #20.
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
- [ ] A contract the graph does not produce is handed back without a
      session (comment on the task, request closed).
- [ ] The view shows the `Contract manager` line and the
      `contract-manager/evolve` prompt.
