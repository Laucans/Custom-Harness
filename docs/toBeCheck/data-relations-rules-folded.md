# Data step relations, entity index, folded rules

## Context
Creation workflow, steps 8 (rules) and 9 (data). Checked on a sample model
mounted by hand in the view; not on a real creation draft.

## Shipped
- `data-model.js`: a click on a table colours what it depends on (blue) and
  what depends on it (magenta), tables and links; UML multiplicities (`*`,
  `1`, `0..1`) on the selection's links only; entity index on the right,
  searched as typed (tables, matching fields), click selects and centres.
- `creation.js`: a folded rule shows its description; Given/When/Then only
  when opened; a "criterion to write" tag when one is missing.

## To verify
- [ ] Step 9 on a real draft: colours and multiplicities readable with 10+ tables.
- [ ] `0..1` at the pointed end for a nullable FK, `1` for a required one.
- [ ] Index fits beside the inspector in the creation easel (no squeeze of the board).
- [ ] The office's data-model pane still reads well with the index (same canvas).
- [ ] Step 8: folded rules show the description; opening one shows Given/When/Then.
