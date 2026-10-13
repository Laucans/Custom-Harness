# The brief's easel, material only

Shipped 2026-10-13, not yet seen in a real creation.

## What shipped

The design system's easel takes a `material` face (`kit.js`, `opts.material`,
passed through `app.js`'s pane registry): the sources alone — inject a tokens
file, or moodboard + logo + design brief — plus the kit's catalogue. No sheet,
no simulated page, no canvas tab, nothing published from it. The creation's
brief step mounts that face (`creation.js`, `EMBEDS.brief`); the design system
step (6) still mounts the easel whole. `EMBEDS` entries carry a `face` so
moving brief → design remounts the pane instead of keeping the other one.

Verified in `harness-view --demo`: mounted both faces, the material one shows
the two source blocks and the callout, zero `.kt-sheet` and zero
`.kt-showcase`; the whole one is unchanged.

## To check in a real creation

- [ ] Step 3 (brief) shows the material easel under the four questions, with
      the catalogue tab reachable and the canvas tab gone.
- [ ] Uploading a moodboard / logo and typing the design brief still saves
      (the *Continue* preparation draws tokens from it).
- [ ] *Generate* from the brief step works and its result is found at step 6.
- [ ] Brief → step 6 and back: the easel changes face each time, no stale pane.
- [ ] A design system already validated: the brief's callout says so, and the
      sheet is only at step 6.
- [ ] The standalone design-system pane (office easel) keeps its remembered
      tab — the brief's face must not write `harness.kit.tab`.
