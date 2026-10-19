# The harness decides where a human was asked

## Context

Every doubt during development waited on a person:
- a session's `AGENT_LOOP_STOP` parked the lane;
- the breaker refused;
- a review its repairs did not clear and the arbiter's hand-offs were left
  under `harness:needs-decision`;
- the refinement's advice did the same.

This is M5 of the plan "autonomous development from the design dossier".

## What shipped

- **The `decide` workflow**, routed right after the rules' task, on any open
  `harness:needs-decision`. It takes the label, then one opus session answers
  `answer`, `merge`, `repair`, `wait` (a blocker on the issue waited for,
  `docs/toBeCheck/wait-for-a-producer.md`) or `cut`.
  - It writes `## Decision` (`decided-by: ai`) and carries the decision out.
  - It labels `harness:ai-decided` (created on boards that lack it) and
    records `DecisionTaken`.
  - An owner's answer (`decided-by: owner`) is never decided over, and costs
    nothing.
- **`repair`** marks the PR with `review::DECIDED_MARKER`, so its repairs
  are counted anew, and poses `harness:pr-fix`.
- **`merge`** on a conflicting PR becomes a `repair`.
- **The dev loop** turns a STOP or a breaker refusal of the `code` stage
  into a question under `## Decision` (`kind: stop|breaker`, `round: n`) plus
  `harness:needs-decision`, and the round goes on (`handed_over`).
  - The third time on one task, a `harness:human` sub-issue blocks it and
    `HumanTaskOpened` is recorded.
- **A task** carrying `needs-decision` is not runnable.
- **`waiting_on_a_human`** reads `harness:human` only. A decision is told
  until `harness:ai-decided` is taken off.

## To check on a real run

- [ ] A session answering `AGENT_LOOP_STOP: …`:
  - the task gets `## Decision` (kind stop, round 1) and
    `harness:needs-decision`;
  - the lane ends 0 and nothing is parked;
  - within two ticks `decide` answers, the task carries `harness:ai-decided`,
    and the loop takes it again with the answer in its SPEC.
- [ ] The breaker refusing: same path, kind `breaker`. The body changed, so
      the next prompt's fingerprint differs and the session runs.
- [ ] A write-side PR blocking after 2 repairs: the notification "repairs:#t",
      then "decided:#t".
  - On `repair`: the PR gets the decision marker and `harness:pr-fix`, and
    `pr_fix` runs once more.
  - On `merge`: `milestone_merge::run` merges it and closes the task.
- [ ] Overrule: write `decided-by: owner` + the answer under `## Decision` and
      pose `harness:needs-decision`. `decide` pays nothing, takes the label,
      and the task goes on.
- [ ] A third stop on one task opens "Decide for #t: …" (`harness:human`,
      sub-issue of the milestone, blocking #t). The notification "#h needs
      you" appears.
- [ ] Cost: one opus session per decision; count them per milestone.
