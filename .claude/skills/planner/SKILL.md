---
name: planner
description: Turn one open harness:roadmap issue into the milestones that deliver it — harness:milestone issues chained with blocked_by, each left for /refinement to flesh out. Writes no document. Use when starting work on a new roadmap item, or when the user says "/planner" or "you are planner".
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
# 5. leave it for /refinement to flesh out
gh issue edit <n> --add-label harness:refinement
```

Long bodies go through `--body-file`, never inline: a body typed on the
command line gets mangled by the shell.

**The milestone body** is a few sentences on what it achieves — `Problem`
and `Goal` is enough. `/refinement` is what expands it into the full
section structure a task's `/business-analyst` pass will read; writing
more here is work `/refinement` redoes.

### Never add `harness:ready`

Nothing you open carries it. The human opens the tap one milestone at a
time (which triggers `split`), and that is the only gate stopping a plan
nobody has read from reaching a paid stage.

### Write non-goals generously

Most milestone failures are scope creep, not bad code. Every deferred item
you name — in the milestone body, or in a `grill:backlog`-style note if
the repo keeps one — is one a later milestone won't quietly absorb.

## 5. Hand off

End by telling the user:

> Milestones #a → #z opened under roadmap #R, chained by `blocked_by`, each
> carrying `harness:refinement`. Nothing carries `harness:ready` — add it
> to the first milestone you want split into tasks, then let `split` (or
> its own skill, if this repo has one) take it from there.

Do not roll straight into fleshing out a milestone's body — that is
`/refinement`'s job, in a clean context.
