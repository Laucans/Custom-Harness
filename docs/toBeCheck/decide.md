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

- [x] A session answering `AGENT_LOOP_STOP: …`:
  - the task gets `## Decision` (kind stop, round 1) and
    `harness:needs-decision`;
  - the lane ends 0 and nothing is parked;
  - within two ticks `decide` answers, the task carries `harness:ai-decided`,
    and the loop takes it again with the answer in its SPEC. (seen 2026-10-24 12:07–18:04 Paris: #486 stopped (stop, round 1), lane exit 0; decide at the next tick labelled harness:ai-decided (wait); once #22 landed the answer came at 18:04 and the loop took #486 the next tick)
- [ ] The breaker refusing: same path, kind `breaker`. The body changed, so
      the next prompt's fingerprint differs and the session runs.
- [x] A write-side PR blocking after 2 repairs: the notification "repairs:#t",
      then "decided:#t".
  - On `repair`: the PR gets the decision marker and `harness:pr-fix`, and
    `pr_fix` runs once more.
  - On `merge`: `milestone_merge::run` merges it and closes the task. (seen PR #493 of #449 → decide `repair` 2026-10-24 12:15 Paris, harness:pr-fix posed, pr_fix ran the next tick; PR #411 of #190 → `merge` 2026-10-23 22:26 Paris, merged and #190 closed; the view's notifications not checked)
- [ ] Overrule: write `decided-by: owner` + the answer under `## Decision` and
      pose `harness:needs-decision`. `decide` pays nothing, takes the label,
      and the task goes on. (contradicted 2026-10-20 06:17–07:55 Paris: on #157 the owner's answer cost nothing (`the owner had decided — nothing paid`), but PR #279's `still blocked after 2 repair(s)` posed the label again — decide re-ran 9 times in 1 h 40, the task did not go on)
- [x] A third stop on one task opens "Decide for #t: …" (`harness:human`,
      sub-issue of the milestone, blocking #t). The notification "#h needs
      you" appears. (seen 2026-10-17 16:51 Paris: #42's third stop opened #74 `Decide for #42: …` (harness:human, blocks #42), HumanTaskOpened recorded; the view's notification not checked)
- [x] Cost: one opus session per decision; count them per milestone. (seen 2026-10-25 01:40 Paris: 133 decide sessions, $40.03 (~$0.30 each) for 131 DecisionTaken; per milestone #16 5, #17 7, #18 16, #19 4, #20 9, #21 6, #22 3, #23 3, #24 5, #25 12, #26 2, #28 54, none 7)
