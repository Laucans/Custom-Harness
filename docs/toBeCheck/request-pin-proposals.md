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

## Fixed since: the duplicated empty pins

A board's `destroy()` emptied its host but left its listeners on it, and
the mock-up pane builds every page's board on the same host: each page
visited left a board that still drew zones and dropped pins — saving them
on *its* page. One zone drawn after two page changes made 5 pins on 2
pages, same millisecond (reproduced with the saves intercepted). Every
listener a board or a pane (mock-up, kit, data model, gallery) sets on what
outlives it now goes with it (an `AbortController`): the same gesture made
1 save, on the page shown.

- [ ] Over a session of page changes and pins, no pin appears on a page
      it was not placed on (`/api/notes`: no empty twins).

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

## Rework with AI and a better informed Propose (`355649b`, `a3968e2`)

Propose now reads the page's objective, the rules the page or its
micro-frontends carry and the mock-up's conventions, says `layout: …`
instead of approximating a layout, and hands a micro-frontend's part over
with the request as pinned. Rework with AI opens a Claude Code session in
the branch's workspace (edit mode), the page's draft written into its
file first; the workspace is kept to put back.

- [ ] Rework on the Fiche d'entité rail: the session reads the page and
      CampaignElements, says its plan, makes a left sticky rail with a
      drawer under 960px; the canvas follows each save.
- [ ] Validate & quit: one history entry "a request reworked with the AI",
      the pin green (or proposed while a draft waits), rules read again.
- [ ] Quit without validating: every file as before the session (page,
      fragments, style.css), nothing left.
- [ ] Discuss it then Rework on the same pin: refused until the first
      session is closed, and the other way round.
- [ ] A Propose on a layout request: the pin says "the AI left it: layout:
      … — Rework with AI can make it", nothing approximated.
- [ ] The first click on a pin sometimes does not open its window (seen
      twice in automation, right after the frame reloads).

## Not fixed

- After each restart the view opens the harness-dnd2 factory: switch back
  to ForgeDesMondes from the factory menu.
