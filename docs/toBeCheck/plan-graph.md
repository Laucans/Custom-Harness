# The plan's graph (contract R) — step 1 of parallel milestones

## Context

The planner chained milestones only on the aggregates they shared; what a
milestone *reads* of another was unsaid, and ForgeDesMondes' #19 (media
library) was planned before #20 (sessions) while reading `session.listScenes`.
The owner's design (2026-10-17): a dependency graph kept in the product,
interface contracts mocked until built, a merge train, a contract manager,
and lanes that serve the critical path first.

## Shipped

- `harness_architecture::graph`: `Graph::derive(milestones, components)`,
  `reads_ahead`, `unproduced`, `critical_path(remaining)`, JSON in and out.
- Contract R `contracts/r-plan-graph.schema.json`, validated by the gates on
  `plan/graph.json` when present.
- The planner names `micro_uis` and `commands` per milestone (`micro-uis:`,
  `commands:` lines under `## Architecture`), writes `plan/graph.json` on the
  integration branch after opening the milestones (`GitHub::put_file`), and
  warns of every read ahead. A milestone written before reads from its text.

## To verify

- [ ] The next planner run (a new roadmap item) writes `plan/graph.json` on
      `main_agent`, one commit, and the gates validate it.
- [ ] Its plan carries `micro-uis:` and `commands:` for every milestone, and
      every dossier component and write lands in exactly one.
- [ ] A read ahead, if any, is in the planner's log as a warning.
- [ ] ForgeDesMondes' current plan (#18–#30) gets its graph: no command
      writes it yet for an existing plan — `harness plan-graph <roadmap>` or
      the watch at start (step 1b).

## Next steps (the design, in order)

1b. A command that (re)writes the graph for an existing plan.
2. Contracts mocked until built: manifests B/F + fixtures at planning, the
   server serving a fixture for an unbuilt id, gate `contract-untouched`.
3. Several current milestones in the watch, lanes serving the critical path
   first, the merge train replacing `main_agent_merge`.
4. The contract manager.
