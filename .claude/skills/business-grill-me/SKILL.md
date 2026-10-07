---
name: business-grill-me
description: Interview the user relentlessly about a project's business objectives — the problem, the user, success criteria, scope boundaries, trade-offs. Use when the user wants to clarify or stress-test *what* a project should do and *why*, before any architecture is discussed.
disable-model-invocation: true
---

Run the `grilling` method (load that skill for the rounds/frontier
discipline) on this subject: **the project's business objectives** — not its
architecture, not its implementation. If the user also wants the technical
side grilled, `technical-grill-me` exists for that and expects this session's
output as one of its own inputs — business decisions come first, because an
architecture grilled before its objectives are settled is architecture for
objectives nobody confirmed.

## Anchor: which project

**This is never about the harness checkout you're running in.** Before
round one, read `TARGET_REPO_URL` from `.env.local` at the harness root
(fall back to the `TARGET_REPO_URL` environment variable). If neither is
set, say so and ask the user for the repository before proceeding — do not
default to grilling about the harness tool itself just because that's the
current directory; it is the instrument, never the subject.

State the resolved `owner/name` back to the user in your very first message
("Grilling the business objectives of `<owner>/<name>` — stop me now if
that's wrong"), so a wrong resolution is caught before a whole session of
questions about the wrong project.

## The subject

Seed the design tree with these branches — not a fixed checklist, a starting
shape; let the user's answers grow or prune it:

- **The problem and who has it.** What's broken or missing today, for whom,
  and how do they cope without this project.
- **Success criteria.** What observable outcome says this worked. Resist a
  vague answer here above all others: "people like it" is not a frontier
  question answered, it's one deferred.
- **Scope boundaries.** What this project explicitly does **not** do, and
  what happens to those needs instead (another project, nothing, later).
- **Constraints.** Timeline, budget, a non-negotiable (a regulation, a
  platform, a stack decision already made elsewhere).
- **Priority when objectives conflict.** Two branches of the tree may turn
  out to want incompatible things — surface that as its own question rather
  than silently picking one.

## The one override to `grilling`'s method

**Facts rule, relaxed.** `grilling` says finding facts is always your job,
never the user's, and to dispatch a sub-agent rather than ask. That holds
for anything checkable — "does a competitor already solve this" is worth a
web search before asking. It does **not** hold for what's only in the user's
head: intent, priorities, who the users actually are, what counts as success.
No sub-agent finds those. Ask directly, and do not substitute a guess
dressed as a recommendation for a question that is actually about the user's
own judgment — `grilling`'s "give your recommended answer" is for questions
where a reasonable default exists, not for ones only the user can answer.

Everything else in `grilling` applies unchanged: rounds, the frontier, one
round at a time, wait for answers, done when the frontier is empty, and —
**do not act on this understanding until the user confirms it.**

## When the frontier is empty

Say so plainly, summarize the shared understanding in a few bullets, and stop.
This skill produces understanding, nothing else — no file, no issue. If the
user wants that understanding turned into `harness:roadmap` issues on the
repository named by `TARGET_REPO_URL`, point them at `grill-to-roadmap`,
which expects exactly this session's outcome as its starting point — and
takes a `technical-grill-me` session as an optional second input, so running
the architecture grilling before it sharpens the item's constraints.
