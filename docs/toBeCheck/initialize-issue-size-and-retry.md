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
- [ ] Rebuild and restart the view, ForgeDesMondes → Initialize again: the
      roadmap task opens (body ≈ 57–63k characters, under 65,536).
- [ ] No second rules task (#2) nor secrets task (#3).
- [ ] `mockup_at` still `main_agent@9e6e4db57f` (or a newer commit), `design_at` kept.
- [ ] The planner reads the lighter roadmap well and puts each rule in a milestone.
- [ ] A forced refusal shows GitHub's reason in the job's error.
