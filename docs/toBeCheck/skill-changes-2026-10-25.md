# Four skill changes, not yet seen in a real round

Shipped 2026-10-25, tested by unit tests only.

## What shipped

- **`/code`, Build section**: three lines adapted from Ponytail — the least
  code the SPEC needs (a `SCAFFOLD:` hole and a mutant-killing test always
  count), lean on dataguard, `platform-test-kit`, `mock/handlers/_lib/`, the
  standard library and what the closure mounts but never on another
  Capability's, DataCapability's or Micro-UI's code, and never drop a
  validation or an error path to stay small.
- **`pr_fix` repair prompt** (`FIX_PROMPT`): reproduce first, one hypothesis
  at a time proven red then green, read what earlier repair rounds proved or
  ruled out in the branch's commits, and write `Cause:` and `Ruled out:` in
  the commit body.
- **Design system easel** (`harness-view`, `domain::design`): an aesthetic
  direction in the tokens prompt and a craft block in the kit component
  prompt (and in the rewrite of a component, unless a request asks
  otherwise), adapted from Anthropic's `frontend-design`. The craft wins over
  a base fragment's autoplay and all-caps labels. Not lent as a skill, so
  no other session sees it.
- **Skill evals** (`scripts/skill-evals.mjs`, `evals/`): `observe` scores the
  factory's real rounds, `run` replays frozen cases.

## Before it reaches the factory

The factory lends the skills of its own harness checkout
(`~/.harness/factories/<f>`), not of `main`: move that checkout to the new
commit for the `/code` lines. The repair prompt and the easel are in the
binaries: rebuild and restart `watch` and the view.

## To check

- [ ] Baseline before the change: `observe <factory> --until <deploy date> --label before`
      (2026-10-24/25: 91 code rounds, pre-flight written 51, stop line 33,
      verification run 87, self-merge 20 — all before the deny rule of
      2026-10-24 12:52).
- [ ] After about 50 code rounds: `observe --since <deploy date> --label after`,
      then `report before after`. No check drops by more than 10 points.
- [ ] A paid replay works end to end: `run split --reps 1` finishes, the
      replays score like the factory's own splits, and no replay reaches GitHub.
- [ ] A round with a Ponytail effect: a code round that names dataguard or the
      kit instead of writing its own helper, and none that imports a neighbour unit.
- [ ] A `pr_fix` round writes `Cause:` and `Ruled out:` in its commit body, and
      a second repair round of the same PR quotes the first round's ruled-out leads.
- [ ] Fewer `repairs_exhausted` events per repaired PR than before (86 in the
      ForgeDesMondes `harness.db` on 2026-10-25).
- [ ] A design system generated from a brief does not fall back on the generic
      looks the direction names (cream and terracotta, near-black and one acid
      accent, one radius and one grey shadow everywhere).
- [ ] A kit component shows a visible keyboard focus, sentence-case labels, and
      no motion with `prefers-reduced-motion`.
- [ ] A Micro-UI code round's transcript never mentions the design direction.
