---
name: technical-grill-me
description: Interview the user relentlessly about a project's architecture — stack, module boundaries, data flow, invariants, open technical risks. Use when the user wants to clarify or stress-test *how* a project should be built, once *what* and *why* are settled.
disable-model-invocation: true
---

Run the `grilling` method (load that skill for the rounds/frontier
discipline) on this subject: **the project's architecture** — not its
business objectives. If `business-grill-me` hasn't run yet for this project,
say so and ask whether to run it first: a technical decision made before the
objectives are settled is a decision for objectives nobody confirmed, and
you'd be grilling architecture against a target that may still move.

## The subject

Seed the design tree with these branches — not a fixed checklist, a starting
shape; let the user's answers grow or prune it:

- **The stack**, where not already fixed by something outside this project's
  control (a platform, a client requirement, an existing codebase).
- **Module boundaries.** What owns what, what depends on what, and —
  explicitly — what must **never** depend on what (the inverse dependency
  that would make a layering rule meaningless if Cargo/the linker/whatever
  else didn't already forbid it).
- **Data flow.** Where state lives, what's the source of truth, what's
  derived and may be stale.
- **Invariants.** The rules a future change must not break, stated so
  precisely a reviewer could check one in a diff.
- **Open technical risks.** What's genuinely uncertain — a library that might
  not do what's needed, a performance assumption untested, an integration no
  one has tried yet. These are *decisions deferred*, not failures to plan:
  name them so `/planner` later sees the dependency rather than the agent
  loop discovering it mid-task.

## Facts stay the model's job — unchanged from `grilling`

This is the ordinary case `grilling` describes: dispatch a sub-agent to read
`CLAUDE.md`, `ARCHITECTURE.md`, the dependency manifest, the actual module
tree — whatever a frontier question needs — rather than asking the user
something the repository already answers. Only ask the user for a
**decision**: a choice between options the repo doesn't already make for you.

## When the frontier is empty

Say so plainly, summarize the shared understanding in a few bullets, and
stop. This skill produces understanding, nothing else — no file is written.
If the user wants durable artifacts from this session (ADRs, a glossary, and
a short digest the harness's own `/planner` reads before opening tasks),
point them at `technical-grill-with-docs`, which expects exactly this
session's outcome as its starting point.
