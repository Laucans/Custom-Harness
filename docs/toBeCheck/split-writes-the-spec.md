# Split writes the SPEC, one session builds each task

## Context

The human opened every tap: `harness:ready` on each milestone and each task,
and `harness:refinement` on each task (15 gestures on one milestone of the
dnd_helper2 run). A task then cost three paid stages (an opus technical
refinement, then `code` and `create-test` on sonnet) plus about seven sonnet
sessions of business refinement. These are milestones M2 and M3 of the plan
"autonomous development from the design dossier".

## What shipped

- `common::dossier` reads `mock/proof-of-concept.json`, the aggregate drafts
  and `rules.json`. The planner gets the whole dossier; split gets its
  milestone's part.
- The planner opens each milestone `harness:ready` (no `harness:refinement`).
  Split writes each task's Business Goal, Acceptance Criteria (rules cited
  `(R<id>)`), Business Rules and Technical sketch, and opens each agent task
  `harness:ready`.
- A slice that is not a whole SPEC gets one retry, then the `SlicesCarryASpec`
  gate stops the split.
- `TaskHasASpec` reads the sections and never asks a human for a label.
- The dev loop has one stage, `code`: plan, build, then tests in the same PR,
  the plan under `## Plan`, `Rules: R…` in the PR body, assumptions recorded,
  `## Decision` honoured. It runs on sonnet, opus on the data layer.
- The `MilestonesPlanned` and `MilestoneSplit` events produce notifications
  that name the gesture to hold one.

## To check on a fresh product

- [ ] The planner logs `design dossier: …` and opens milestones with
      `harness:ready` and no `harness:refinement`. The notification
      "#R: N milestone(s) planned" appears.
- [ ] Split opens tasks carrying the four sections, each criterion with its
      rule. Agent tasks carry `harness:ready`; human tasks do not. Count the
      tasks with all four sections: expected all of the agent ones.
- [ ] No hand label is posed between Initialize and the first PR.
- [ ] A task runs one `code` session: `costs.tsv` has one row per task, no
      `technical-refinement` or `create-test` rows. Its PR carries the tests,
      `## Plan` and `Rules:`.
- [ ] Cost per task compared with the three-stage baseline of the dnd_helper2
      run (healthy task $2–5): target a median of at most $2.5.
- [ ] A task opened by hand without the sections stops with "carries no SPEC".
      Writing the sections lets the next tick take it.
