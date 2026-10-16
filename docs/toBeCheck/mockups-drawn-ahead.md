# The proof of concept is drawn ahead, from the scope's validation

## Context
The mock-ups, their micro-frontends (Capabilities, DataCapabilities) and the
requirements waited for the human to reach the mock-up and review steps.
They now draw in the background as soon as the scope is settled, while the
human works on the design system.

## Shipped
- `CreationDesk::draw_ahead` (`POST /api/creation/ahead`): draw →
  componentize → requirements, one job; the human's step is not moved.
- `Creation::mocked_scope` / `mockup_stale`; `Creation::drawn` resets
  `componentized` on a new drawing.
- `creation.js`: auto-start once the record is past the scope and no job
  runs (after the preparation ends); mockups crumb chips; the mock-up step
  waits (preview + checklist) while the micro-frontends are made.

## To verify in a real creation
- [ ] Scope → *Continue*: within seconds the mockups crumb shows *drawing*,
      then *ready* while the review's crumb shows *micro-frontends*; the human
      stays on the design step.
- [ ] Preparation still running at that moment: the drawing starts once it ends.
- [ ] Mock-ups drawn before the kit is generated: pages pick up the tokens
      once validated (`var(--…)`); note which kit components are missing.
- [ ] Reaching the mock-up step while the micro-frontends are made: preview
      + checklist, no editable easel until done.
- [ ] Reaching the review: requirements already there, tied to pages and
      micro-frontends, no new session started.
- [ ] Back to the scope, edit it, *Continue*: *to redraw* chip and the
      callout on the mock-up step; *Draw again* follows the new scope.
- [ ] Time saved: from design validation to review, compare with a creation
      before this change.

## The mock-up's manifest (contract N)
Shipped: the exact `mock/pages.json` shape in the mock-up rules (written
from the view's own types), a repair session when it does not read (2 at
most, then the job stops instead of ending done), and `fixture` as one
file or a list (schema N and `MockComponent`).

- [ ] A drawing whose micro-frontend reads several fixtures: listed, the
      easel reads it, the product's gate `validate n-mock-pages` passes.
- [ ] A drawing that leaves `pages.json` unreadable: the job's steps show
      "… — a session repairs it", then the easel opens.
- [ ] Products installed before: `init-repo` refreshes the schema
      (`contracts/n-mock-pages.schema.json`) — check the product gets it.


## Reference page, then the other screens in parallel
Shipped: `drawn` draws the scope's first `## Screens` bullet alone (the
reference), then the others `--creation-sessions` (4) at a time on its look;
the desk writes `mock/pages.json` and `mock/index.html` itself
(`domain/drawing.rs`, `list_pages`).

- [ ] A scope of 5+ screens: the reference page's step shows first, then the
      others arrive 4 at a time; total time against a single session.
- [ ] The other pages really share the reference's menu, header and spacing.
- [ ] A scope edited with no `## Screens` section: the step fails with a
      readable reason instead of drawing nothing.
- [ ] A drawn-again mock-up: pages of screens dropped from the scope are gone
      from `pages.json` and the workspace; the pins on them are put away.
- [ ] `mock/index.html` opens and links every page; the unknown-variables
      step does not flag it.
