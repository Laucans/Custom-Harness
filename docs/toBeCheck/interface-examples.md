# Interface examples in the design system

## Context
The user wanted optional interface examples Claude takes after when it
generates the product: the structure (layout, navigation, density), never the
look (the tokens keep the last word) nor the texts.

## What shipped
- `Design.examples` (contract O, `harness-architecture::kit::Example`), at most 6.
- Easel block "Interface examples" (material face and step 6): upload, scope
  (whole app / a page / a kit component), note, remove.
- Kit generation reads its component's and the whole app's examples.
- Mock-up drawing and the cut into micro-frontends read the whole app's and
  the pages': lent under `design/examples/`, taken back after.

## To verify for real
- [ ] Upload 2–3 screenshots on forgedesmondes, one per scope; the note and
      the scope survive a reload.
- [ ] Draw again: the job lists "N interface example(s) … lent", then one
      `example <file>: …` line per example.
- [ ] The drawn pages follow the examples' structure, with the design
      system's colours and fonts, not the screenshots'.
- [ ] After the drawing, no `mock/design/examples/` left on `main_agent`.
- [ ] Regenerate a kit component with a component-scoped example: its
      fragment follows it.
