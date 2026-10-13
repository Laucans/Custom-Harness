# Founding a factory from one whose creation is half-done

## Context
From a factory still being created, *+ new factory* → *Initialize a factory*
→ *Start* landed on the **existing** factory's repository step ("the
repository X is created… Continue"), so the human walked back into that
factory's own creation instead of founding a new one. The founding form was
unreachable as soon as the factory open in the view had a creation record.

## What shipped
- `crates/view/static/creation.js`: a `found` route
  (`#new-factory/found`) that renders the founding form whatever the desk's
  record holds, read as a factory with nothing done (crumbs: step 1 here,
  2–9 locked), with a callout naming the factory it is founded beside.
  `#new-factory/repository` keeps the record's own step.
- `crates/view/ARCHITECTURE.md`: the step 1 paragraph says so.

No Rust change: `POST /api/creation/found` never looked at the record.

## Checked
Only in the browser against `harness-view --demo` with `/api/creation`
stubbed to answer a mid-creation record: *Start* opens the form, the crumbs
stay locked, *Back* and *Resume* return to the existing creation.

## To verify for real
- [ ] From a factory mid-creation (a record at, say, the grilling step),
      found a new factory: the form opens, `gh repo create` runs, the job
      panel reports `init-repo`, and the view **switches to the new
      factory** — the header's factory button shows the new name, the page
      opens on step 2 (*kind*) of the new factory.
- [ ] The factory left behind keeps its record: reopen it from the menu, it
      resumes at the step it was on, nothing lost.
- [ ] Reload on `#new-factory/found` from that same factory: the form comes
      back, not the record's step.
- [ ] *Connect an existing repository* from the same landing still works.
