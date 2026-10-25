# Initialize: issue size and retry from a failure

## Context
ForgeDesMondes' initialize failed: `GitHub refused to create an issue …
Validation Failed (HTTP 422)`. The roadmap task's body inlined the whole
record (~90k characters); GitHub takes 65,536. "Initialize again" then
failed on "nothing to save: the mock-up is as committed": the first attempt
had published, the design carry moved `main_agent`, the retry's commit was
refused as not a fast-forward and left nothing to save aside.

## Shipped
- `ISSUE_BODY_MAX` in core's GitHub port; `GhCli::create_issue` refuses a
  longer body before sending; every refused write says GitHub's own reasons.
- Roadmap task lighter: the rules counted, pointing at the rules' task and
  `rules.json` (each milestone names its rules); `mock/proof-of-concept.md`
  keeps the whole record. Still too long: the longest sections fold to a
  pointer at that file (`fit_issue_body`).
- `GitMockups::publish`: a refused push is rebased on the branch and pushed
  again; failed, the change is left uncommitted for a save aside.
- Retry keeps what the first attempt did: `mockup_at`, `design_at`,
  each secrets task as soon as it opens.

## To verify
- [x] Rebuild and restart the view, ForgeDesMondes → Initialize again: the
      roadmap task opens (body ≈ 57–63k characters, under 65,536). (seen 2026-10-17 00:05 Paris: roadmap task #4 opened, body 47,685 characters — under 65,536, below the expected range since Decisions folded)
- [x] No second rules task (#2) nor secrets task (#3). (seen 2026-10-17 00:05 Paris: the retry opened #4 only; #2 and #3 are the first attempt's (22:04), #5+ the planner's)
- [x] `mockup_at` still `main_agent@9e6e4db57f` (or a newer commit), `design_at` kept. (seen 2026-10-17 00:05 Paris: #4 names `main_agent@9e6e4db57f`)
- [x] The planner reads the lighter roadmap well and puts each rule in a milestone. (seen 2026-10-17 03:16 Paris: the planner opened milestones #16–#30; all 49 rules of rules.json are named in their bodies)
- [ ] A forced refusal shows GitHub's reason in the job's error.
