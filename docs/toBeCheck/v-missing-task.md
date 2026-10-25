# V: no declared task dropped, no decision that opens nothing

## Context

ForgeDesMondes #219 (milestone #28), 2026-10-25 (`harness-dnd2-run/DECISIONS.md`
08:50, `night/priorities-H.md` entry V). About $2.46 in code sessions plus 4
decide sessions:

- 07:04: the stop declared 2 slices. Only #535 (Micro-UI PreviousSummary) was
  opened. The fixtures slice (`"unit": "plumbing"`, `"produces": []`) was
  unwritable, and with `--no-human-gate` it was kept silently in the wait's
  `## Decision`.
- 07:38: the stop declared 2 slices, both with `"unit": "server-fixtures"`.
  Both were unwritable, so a decision was asked. decide answered "the
  fixtures task is approved: it becomes a write-side task of milestone #28
  that blocks #219" (rounds 1–4) but opened nothing. #219 reran and stopped
  again. #537 and #538 were opened by hand.

Now:

- V1: `plumbing`, `fixtures`, `server-fixtures`, `seed-data`… are read as
  `infrastructure` (write side), and the opened task's brief says so.
  `infrastructure` and `tooling` may produce no id. A same `branch:` is the
  same task.
- V1: a slice declared wrong (unknown unit, a field missing, unreadable
  JSON) is refused back to the session. A comment on the task names the
  slice, why, and every unit key. The `## Decision` is `kind: refused` (or
  the wait, with the note under it). The code prompts list the unit keys and
  say what `refused` means. After 3 rounds it goes to a person, or to decide
  with no human gate.
- V2: decide has a `task` action, offered only for a `kind: stop` question
  carrying `declared:` and `milestone:`. Its `tasks` slices are opened by the
  stop handler's own opener (`common::missing_task`), and the task is
  blocked on them. An `answer` that approves a task in words does not parse.
  The same `declared:` after an AI ruling never asks a second decision:
  the refusals carry the ruling (`ruled:`, `ruled-at:`), and past 2 of them
  the next identical stop is a person's, gate or none.
- Review fixes: a slice whose `branch:` a closed sibling carries is never
  opened again (stop handler and decide's `task`); it is refused back, a
  round. A slice with no branch is keyed by a comma-free slug and hash, so
  `declared:` reads back. An answer is refused as "approval in words" only
  when a clause approves a task.

## What shipped

- `fix(v): a declared missing task is never dropped; decide's approval opens
  it` on `fix/v-missing-task` (off `fad5564`), merged into `main` as `Merge V:
  missing tasks are never dropped; decide's approval opens them`. See
  `git log -- crates/workflows/src/common/missing_task.rs`.

## To verify

- [ ] The factory restarted on this harness. `.llocal/prompts.json` has no
      override of `agent-loop/code`, `agent-loop/code-write-side` or
      `decide/decide`. If it has one, the old text runs: no unit keys, no
      `task` action.
- [ ] On the next missing-task stop (grep run.log for `for the missing task`):
      every declared slice is either opened (`opened #… for the missing
      task`) or refused (`the declared task "…" is not opened — …`). The
      count of slices in the stop line equals opened + refused.
- [ ] A slice whose unit is `server-fixtures` / `plumbing` / `fixtures`
      opens as `unit: infrastructure` with `harness:write-side` and
      `harness:data-layer`. Its Scope ends with "The stop declared its unit
      as `…` … on the write side".
- [ ] A refused slice: the task gets a comment starting
      `**Declared tasks the harness did not open**` that lists the slice and
      the unit keys. `## Decision` is `kind: refused`, with
      `verdict: refused — re-declare N declared task(s)`. No
      `harness:needs-decision` label. The next session's stop re-declares it
      with a valid unit (expected: the round 2 of the stop, `round: 2`, opens it).
- [ ] With no human gate, a decision asked on a missing-task stop has
      `milestone: #M` and `declared: feat/…` in its `## Decision`, and the
      stop is quoted whole under `The stop, whole:`.
- [ ] decide's `task` answer: the journal shows `opened #… for the missing
      task` from the decide run, and the task is blocked on it. decide's
      comment ends with "The approved tasks are #…; blocked by them until
      they land". The task does not run again until they land.
- [ ] No issue is opened twice: a decide `task` on a slice already opened is
      "parked on it". A re-declared slice with a reworded title but the
      same branch is found.
- [ ] The same declaration stopped on again after a decide ruling: no second
      `harness:needs-decision`, on any later identical stop. The comment
      quotes "the harness decided on it already: …", and `## Decision` keeps
      `ruled:` and `ruled-at:`. The third identical stop after the ruling
      opens a "Missing task for #N" `harness:human` issue (expected: rounds
      ruled+1 and ruled+2 refused, ruled+3 to a person).
- [ ] A slice re-declared after its task landed (same `branch:`, task
      closed): no new issue. The comment says "#N built `feat/…` already
      (closed): build on it, or declare a new branch".
- [ ] A vague stop (`AGENT_LOOP_STOP: task <prose>`): `declared:` holds one
      key like `the-projection-1a2b3c4d`, no comma inside.
- [ ] Cost check on the next #219-like case: one code session and no decide
      session before the missing tasks are opened (expected under $1, against
      ~$2.46 + 4 decides on 2026-10-25).
