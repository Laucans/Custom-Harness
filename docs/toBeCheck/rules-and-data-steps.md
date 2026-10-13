# The review becomes the rules step; a data step before the initialization

## Context
The creation's step 8 (review) did not let the human judge what the
factory would build: a flat list of requirements with every page and
micro-frontend as a chip, no coverage, nothing of the architecture. After
a grilling (2026-10-13) it became **Rules** (rules by family, criteria,
coverage, a rule picked shown on the easel, the architecture's guard),
and a **Data** step came between it and the initialization.

## Shipped
- `1b9e648` the easel's micro-frontends list in two sections, lit both
  ways; hidden on the mock-ups step.
- `9cc9fd1` the crumbs' marks (done, stale, blocked, open, locked).
- `e41310a` the rules step (family, criterion, validation, reread one).
- `8d7d89c` the architecture's guard and the simple view.
- `35004e4` the data step; the acceptance moved there.

Tested by unit, desk and server tests and by the page against a live view
with the AI calls intercepted. **No real AI call was made**: every prompt
below is untried on a real model.

## To verify in a real creation
- [ ] Rules read by the AI: each has a `family` and a `criterion` written
      "given…, when…, then…", in the brief's language; 8–25 of them.
- [ ] ↻ on a rule: only that rule changes, its number kept.
- [ ] A rule picked: the easel's list shows only its micro-frontends (two
      sections), dashed outlines on the page, its first page shown; a page
      chip shows that page.
- [ ] ↻ on a micro-frontend from a rule: its fragment is generated again
      (watch the generations in the easel's micro-frontends view).
- [ ] Hover both ways in the list: the part lights on the page; a part
      under the pointer (also in *interact* mode) lights its entry.
- [ ] The guard runs once by itself on entering the rules step: findings
      in the brief's language, effects for the writes; note how many
      `write_as_read` it reports on a real cut (false positives?).
- [ ] *apply* on a finding: the session changes `mock/pages.json` and the
      fragments, keeps every micro-frontend's name; the easel reloads.
- [ ] A record from before (step `review`, no `visited`, no `rules_for`):
      opens on the rules step, not stale, its rules have no criterion yet
      (red) — *Read all again* fills them.
- [ ] The data step drafts by itself on entering: tables, aggregates,
      invariants citing the business rules; the diagram draws the tables.
- [ ] *Ask the AI* changes the structure and keeps the policies.
- [ ] Criticality and policy selects, purpose fields: kept at once, the
      validation taken back.
- [ ] Accept: `mock/data/model.json` and `mock/data/aggregates.json` on
      the integration branch beside `mock/proof-of-concept.json`; the
      roadmap task has `## Rules` (family, criterion) and `## Data`.
- [ ] Back to the rules, change a criterion: the Data crumb turns yellow.
