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
- [x] ForgeDesMondes' current plan (#18–#30) has its graph: `harness
      plan-graph 4` wrote it on `main_agent` (15 milestones, 123 ids, 5 reads
      ahead: #19 → #20 ×3, #20 → #25 ×2).
- [x] The gates validate `plan/graph.json` against contract R on the next
      push of `main_agent`. (seen 2026-10-24 22:06 Paris: `gates` green on main_agent a3823e4 `chore(plan): milestone #24 built 6`, which changes plan/graph.json; manifests-conform-to-contracts runs `validate r-plan-graph.schema.json plan/graph.json`)

## Next steps

Steps 2–4 shipped: see `parallel-milestones.md`.
