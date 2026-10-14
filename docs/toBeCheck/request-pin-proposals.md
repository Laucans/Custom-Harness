# Request pins proposed on their micro-frontend, rules following

## Context

A request pin on a mock-up page did nothing until its buried "Apply to the
draft" button was pressed, then edited the page's HTML even when the pin sat
on a micro-frontend, with no visible outcome and no follow-up on the rules.

## What shipped

- `565d5e3` — on a page, **Propose** drafts the micro-frontend under the pin
  (`/api/mockup/mf/{name}/apply`), or the page when none is; the page shows
  the draft at once (a fetch shim in the frame loads `<Name>.draft.html`).
  A blue bar above the page settles each draft: changed everywhere, forked
  for this page (the page injects the fork), or discarded. Kept, the canvas
  emits `harness:mf-saved`; `creation.js` posts `/api/creation/rules/propose`
  and the AI's proposals wait on the record, accepted or refused in Rules.
  Pin window: note and request only, draggable by its head, says what the
  AI does and what it came to. Drafts no longer count as micro-frontends.

## To verify (ForgeDesMondes, page Paramètres IA, AiSettings drafted already)

- [ ] **Validate** in the blue bar: AiSettings' fragment takes the draft, the
      bar goes, the pin goes, a toast says so.
- [ ] Within ~1 min, a callout on step 7 says "N changes to the rules
      proposed"; Rules shows them with Accept / Refuse, each beside the rule
      as it is now.
- [ ] Accept one: the rule is rewritten (or added), "validated" drops.
      Refuse one: it goes, the rule unchanged.
- [ ] On a micro-frontend used by 2+ pages: **Only on this page (fork)** →
      `components/<Name>2.html` exists, this page injects `<Name>2`, the other
      pages still show `<Name>`; the proposals tie the page's rules to the fork.
- [ ] **Discard**: the draft goes, the pin turns back to "not proposed yet".
- [ ] A pin on no micro-frontend (page chrome): Propose edits the page's draft
      and says how many changes.
- [ ] The pin window dragged aside stays where it was dropped while the AI
      works; it opens next to the pin again for another pin.

## Not fixed

- Empty request zones duplicated on 3 boards (campagne, parametres-ia,
  preparation-seance) with identical timestamps: origin not found yet.
