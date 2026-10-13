# The brief, told at large then split

Shipped 2026-10-13, not yet seen in a real creation.

## What shipped

Step 3 (brief) opens on one description textarea (`Creation::description`).
*Split into the 4 parts* sends it to the AI (`POST /api/creation/decompose`,
`decompose_prompt` / `parse_brief`), which writes the brief under the
`BRIEF_PARTS` headings; the four fields then show, prefilled, and *Continue*
appears. The parts' titles now come from the server (`briefParts` in
`GET /api/creation`), no longer hard-coded in `creation.js`.
Auto split: the description left (focusout) for another part of the page for
10 s, before the first split only; cancelled by another tab/window
(`document.hidden`, `!document.hasFocus()`) or by coming back to the field.

## To check in a real creation

- [ ] A new factory's brief shows the description alone, no *Continue*.
- [ ] *Split* fills the 4 fields in the description's language, nothing
      invented; a part not mentioned stays empty.
- [ ] Write, click into the design system's material (moodboard, design
      brief) and wait 10 s: the split starts on its own.
- [ ] Write, switch to another tab or window for 10 s+: no split on return.
- [ ] Clicking into an iframe of the easel (if any) does not cancel the
      countdown (`document.hasFocus()` true in Chrome / Safari).
- [ ] After a split, editing the description never re-splits on its own;
      *Split again* over edited parts asks a second click.
- [ ] *Fill them myself* shows the empty parts and *Continue*.
- [ ] A record from before (brief, no description) opens with its 4 parts.
