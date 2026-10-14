# Request pins: proposals, rules in place, pin Undo, Cmd+Z history

## Context

A request pin on the mock-up drafts the micro-frontend it sits on and
reads the rules it touches; the human validates, forks or takes back; a
linear Cmd+Z history covers pins, rules, requests and retouches.

## What shipped

- `565d5e3` — Propose drafts the micro-frontend under the pin, shown live;
  the blue bar settles it (everywhere / fork for this page / discard).
- `8b0bae3` — rules proposed at Propose, shown in place (dashed orange),
  Validate / Refuse; per-pin steps (`StepStore`), pin Undo, undo & delete,
  the lock badge; validated pins stay (green).
- `d5c87e2` — an open pin outlines the micro-frontends its zone touches and
  filters the rules by them.
- `f8c88d4` — the mock-up's history (`HistoryStore`, 30 a branch): Cmd+Z /
  Cmd+Shift+Z out of a field, the board's undo/redo; a question in the page
  before going to another page or to the rules.

## Checked live (2026-10-14, ForgeDesMondes, Paramètres IA)

- Propose: draft in 35 s, 3 rule proposals 20 s later (R3, R77 rewritten,
  one new), cards dashed orange, filter "pin: AiSettings" 2 of 41.
- Lock badge on the locked words; pin Undo (via its route) put back the
  draft, the 3 proposals and the pin state.
- Cmd+Z of "a pin placed" from another page: question, view went back,
  pin gone; Cmd+Shift+Z there without a question; Cmd+Z again.

## To verify

- [ ] **Validate** in the blue bar: fragment replaced, the pin turns green
      and stays, its proposals accepted (cards back to normal), one entry
      "a proposed change validated" in Cmd+Z.
- [ ] Cmd+Z right after that validation: fragment, rules and pin as before.
- [ ] **Only on this page (fork)** on a micro-frontend used by 2+ pages:
      `<Name>2` injected on this page only, its rules retied to the fork.
- [ ] Undo & delete on a validated pin; plain delete keeps the change.
- [ ] A rule's title edited in Rules, then Cmd+Z out of the field: the
      title comes back; typing in one field makes one entry.
- [ ] Cmd+Z of a rule change while on step 7: question, view goes to Rules.
- [ ] Edit mode retouch (move a part), Cmd+Z: the page's draft as before.
- [ ] A redo of a pin: shown "detached" a moment while the frame reloads —
      check it reattaches; if not, reload the board after the frame loads.
- [ ] Something changed outside the history (Read all again, Draw again):
      Cmd+Z says it "moved since" and leaves it.

## Not fixed

- Empty request zones duplicated on several boards with identical
  timestamps: origin not found (cleared by hand on 2026-10-14).
- After each restart the view opens the harness-dnd2 factory: switch back
  to ForgeDesMondes from the factory menu.
