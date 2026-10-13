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
      then *micro-frontends*, then *ready*; the human stays on the design step.
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
