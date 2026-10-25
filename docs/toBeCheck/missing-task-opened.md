# A missing task is opened by the harness, not handed to a human

## Context

ForgeDesMondes, 2026-10-22: #191 (`table.getBattlemapInstance`, milestone
#25) could not be finished without a `ProjectPg` on the `BattlemapInstance`
aggregate, which no task of #25 built. The session followed the code prompt
("anything a human must do first becomes its own `harness:human` issue") and
opened #405 itself: `harness:human`, sub-issue of #25, blocking #191. The
owner had nothing to decide — the session had said what to build, in which
unit, and why.

Now the code prompt says a task the plan lacks is not a human's: the session
stops with `AGENT_LOOP_STOP: task <one JSON slice, split's shape>`, and the
dev loop's stop handler (`dev_loop::action::stop`, `dev_loop::data::missing`)
opens it with split's renderer as a `harness:agent` + `harness:ready`
sub-issue of the milestone, or reuses the open agent sub-issue with the same
title or id, and parks the stopped task on it (`blocked_by`, `kind: wait`).
A slice that is not whole goes to a `harness:human` "Missing task for #N"
saying why (a decision without a human gate). Green tests only: no session
has stopped this way yet.

## What shipped

- The commit `feat(dev-loop): a stop on a missing task opens it under the
  milestone` (see `git log -- crates/workflows/src/dev_loop/data/missing.rs`).

## To verify

- [x] #405 is the reference case. It is open, `harness:human`, and blocks
      #191, which is closed (delivered by PR #407). It is not reused by the
      new path (only `harness:agent` sub-issues are): the owner either
      closes it and lets the next stop on the projection open the agent
      task, or turns it into the agent task by hand. The test fixture
      `missing::BATTLEMAP` is the stop #191's session would have written;
      the body it renders is the shape to expect. (seen 2026-10-24 01:16 Paris: #405 was turned into the agent task by hand (labels harness:agent, ready, write-side, data-layer), built by the loop, closed 2026-10-23T23:16Z)
- [x] On the next real stop of this kind (grep the watch journal for
      `opened #… for the missing task`): the stop line is one line, the JSON
      reads, and the new issue carries `## Scope` with `branch: feat/…` and
      the line `Opened by the harness for #N`, `## Architecture` with
      `unit:`, `id:`, `system:`, `produces:`, `side:`, then `## Business
      Goal` and `## Acceptance Criteria`.
- [x] Its labels: `harness:agent`, `harness:ready`, and
      `harness:write-side` + `harness:data-layer` for a data-layer unit
      (an aggregate's projection), `harness:read-side` otherwise.
- [x] It is a sub-issue of the stopped task's milestone, and the stopped
      task is `blocked_by` it, with `## Decision` `kind: wait`,
      `verdict: wait — until #M lands`; no `harness:needs-decision`.
      Checked 2026-10-24: #452 (milestone #22) opened #473 (Entity
      hydrate) — Scope/branch/"Opened by the harness", Architecture with
      unit/id/system/produces/side, Goal and Criteria; agent + ready +
      write-side + data-layer; sub-issue of #22; #452 blocked_by #473,
      `kind: wait`, no needs-decision.
- [x] The loop takes the new task (its SPEC passes `TaskHasASpec`, its
      closure starts from `produces:`), and once it lands
      (`waiting-merge` or closed) the stopped task is taken again. (seen 2026-10-24 10:02 Paris: #473 ran 07:20Z and closed 08:00Z; the loop took #452 again at 08:02Z)
- [x] A second stop naming the same task while it is open opens nothing:
      the journal says `the missing task is #M already — parked on it`. (seen 2026-10-24 09:14 Paris: run 20261024-071156-448 `#448: the missing task is #473 already — parked on it`)
- [ ] A session that writes a vague `AGENT_LOOP_STOP: task …` (no JSON, or
      a slice without goal/criteria/unit/system/produces/branch) gives a
      `harness:human` "Missing task for #N: …" whose body says why.
- [x] Sessions do write the JSON on the stop line, as the prompt asks
      (`CODE`, `CODE_WRITE_SIDE`), rather than opening a human issue. If a
      factory keeps a prompt book (`.llocal/prompts.json`) with an older
      copy of the code prompts, the new paragraph is not in it. (seen 2026-10-24 20:00 Paris: six JSON stop lines (#452 x2, #448, #449, #486 x2) opened #473, #474, #477, #510, #512; no human issue opened instead; the factory has no `.llocal/prompts.json`)
