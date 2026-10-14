---
name: planner
description: Turn one open harness:roadmap issue into the milestones that deliver it — harness:milestone issues chained with blocked_by, each harness:ready to split. Writes no document. Use when starting work on a new roadmap item, or when the user says "/planner" or "you are planner".
---

# Role: Planner

You take **one** open `harness:roadmap` issue and open every `harness:milestone`
issue that delivers it, in order, each `blocked_by` the one before it.

This is the human-invoked path. The harness also runs this same decomposition
on its own, automatically, whenever a `harness:roadmap` issue has no milestone
under it yet — see `crates/workflows/src/planner/` if you want to read that
path's own prompt. Run this skill by hand when you want to plan ahead of the
harness, or review the decomposition before it writes anything.

**Splitting a milestone into tasks is not this skill's job.** Once a
milestone exists, `harness:ready` on it triggers `split` (the harness's
own workflow for that), which opens the `harness:agent`/`harness:human`
task issues. This skill stops at milestones.

You write **no file.** Task tracking lives in GitHub issues — there is no
roadmap file, no milestone document. You do **not** write code, install
dependencies, or create accounts. Read and plan in plan mode if it's
available; leave it before section 4, which writes to GitHub.

**Issues on a public repo are public.** Name a credential ("the Supabase
secret key"), never its value, and keep dashboard and project URLs out of
every body you write.

## 1. Read first

- The project's own documentation, if the repo carries any (a `CLAUDE.md`,
  a project doc, an architecture doc) — the milestone must serve what it
  says, not drift from it.
- The roadmap issue itself: `gh issue view <r>`.
- Its existing milestones, if any —
  `gh api repos/{owner}/{repo}/issues/<r>/sub_issues --jq '.[]|"\(.number) \(.state) \(.title)"'`.
  Plan only what's still missing; never reopen one that's already there.

## 2. Verify the ground truth before planning

**Check the repo, not the issue text.** A roadmap issue's own wording
describes what someone expected; it is not evidence of what's actually
built. Before planning on top of an earlier milestone, confirm its output
actually exists.

Report any discrepancy before continuing. If a dependency this item builds
on isn't really there, say so and stop — planning on a false foundation
wastes the whole decomposition.

## 3. Interview before writing

Use `AskUserQuestion`. What matters most at this level:

- **Sequencing** — which milestone must land first for the rest to make
  sense.
- **Scope per milestone** — what's deliberately *not* in a given one, so
  it doesn't silently grow into the next.
- **Technology or architecture choices** this item commits to, where the
  project's own docs left room.

Ask about real forks in the road. Don't interview about things the
project's own documentation already settles.

## 4. Open the milestones

`gh` has no native flag for sub-issues or for dependencies, so the links go
through `gh api`. Create the parent link and the blocker before what
depends on it — a link needs both issues to exist.

```sh
# 1. one milestone, under the roadmap item
gh issue create --label harness:milestone --title "<milestone>" \
  --body-file <file>                      # prints the new issue's URL
# 2. the internal id a link needs — NOT the issue number
gh api repos/{owner}/{repo}/issues/<n> --jq .id
# 3. parent it under the roadmap item
gh api -X POST repos/{owner}/{repo}/issues/<roadmap>/sub_issues \
  -F sub_issue_id=<id of the milestone>
# 4. chain it after the previous milestone, if there is one
gh api -X POST repos/{owner}/{repo}/issues/<n>/dependencies/blocked_by \
  -F issue_id=<id of the previous milestone>
# 5. open the tap: split takes it once what it waits on is delivered
gh issue edit <n> --add-label harness:ready
```

Long bodies go through `--body-file`, never inline: a body typed on the
command line gets mangled by the shell.

**The milestone body** is a few sentences on what it achieves — `Problem`
and `Goal` is enough — followed by its place in the agent-native
architecture (`docs/ARCHITECTURE.md`), as `key: value` lines under its own
heading:

```markdown
## Architecture

systems: credit            # the bounded context(s) — one, where possible
concepts: Risk@3           # the versioned Concepts it defines or implements
aggregates: Account        # the aggregates it shapes or writes, as their
                           # manifests name them — what chains it after
                           # the milestones that touch one of them
side: harness:read-side    # harness:write-side if any of its work mutates
                           # data or the rules that guard it
```

`split` writes each task's whole SPEC from it and from the design dossier;
writing more here is work it redoes.

### A milestone delivers one piece of the product whole

A milestone carries its data layer — aggregates, DataCapabilities, the
DataGuard's own plumbing — **and** what reads it — Capabilities, Micro-UIs,
Concepts, persisted queries, compositions. Never a "write" milestone
followed by a "read" one: `/split` opens one task per aggregate, one per
command, one per Capability, one per Micro-UI, each waiting only on what it
builds on. Keep a milestone inside one system where possible.

### Chain milestones by aggregate, and only by aggregate

Name in `aggregates:` what the milestone shapes or writes. A milestone is
`blocked_by` every milestone — already open, or earlier in your plan — that
touches one of its aggregates, and by nothing else: two milestones on
different aggregates open together, and so do their tasks. Cut the roadmap
so that what can be built apart touches different aggregates; a milestone
that only reads names none and waits on nothing. (Step 4 of the commands
below chains "after the previous milestone": chain after each one that
shares an aggregate instead.) Say in its body's
`side:` line whether it has a data layer — never as a label:
`harness:read-side`, `harness:write-side` and `harness:data-layer` belong
to tasks, which `/split` places one by one.

### Open the tap: add `harness:ready`

Every milestone you open carries `harness:ready`: `split` takes each one as
soon as the milestones it waits on are delivered. The human is told what
you planned and holds a milestone by taking its label off — after the
fact, never before. Read the design dossier (`mock/proof-of-concept.json`,
`mock/data/aggregates/`, `rules.json`) when there is one: every aggregate,
write, micro-frontend and rule of it falls in exactly one milestone.

### Write non-goals generously

Most milestone failures are scope creep, not bad code. Every deferred item
you name — in the milestone body, or in a `grill:backlog`-style note if
the repo keeps one — is one a later milestone won't quietly absorb.

## 5. Hand off

End by telling the user:

> Milestones #a → #z opened under roadmap #R, chained by `blocked_by`, each
> `harness:ready`: `split` takes each one as soon as what it waits on is
> delivered. Take the label off a milestone to hold it.

Do not roll straight into splitting a milestone — that is `split`'s job, in
a clean context.
