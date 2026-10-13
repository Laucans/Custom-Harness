# The POC scope is proposed after the grilling

## Context
The preparation (brief → grilling) drew the tokens, the kit **and** a first
scope from the brief alone. The scope now waits for the grilling's decisions:
it is proposed on entering the scope step (`propose_scope`).

## Shipped
- `CreationDesk::prepared` draws the design system only.
- `creation.js`: no scope in the preparation; *proposing* chip and callout
  follow the scope call itself.

## To verify in a real creation
- [ ] Brief → *Continue*: the job's steps name the tokens/kit, never a scope.
- [ ] During the grilling, the scope crumb shows no *proposing* chip.
- [ ] Grilling over → *Continue*: the scope step shows the callout, then a
      scope that reflects the grilling's answers.
- [ ] Back to the grilling, answer moved, scope not retouched: re-entering
      the step proposes it again.
