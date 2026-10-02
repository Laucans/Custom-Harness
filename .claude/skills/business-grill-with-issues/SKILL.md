---
name: business-grill-with-issues
description: business-grill-me, then turns the result into harness:roadmap issues on the target repository, plus the business digest /planner reads. Use when the user wants the business interrogation to produce a populated backlog, not just shared understanding.
disable-model-invocation: true
---

Call the Skill tool with `business-grill-me` first, unless this same
conversation already ran it and nothing has been added since — in that case,
use its result directly rather than re-grilling. Do not take the step below
until the user has confirmed the shared understanding: `business-grill-me`
itself says so, and writing to GitHub on an unconfirmed understanding is
worse than writing nothing.

## Find the target repository

Read `TARGET_REPO_URL` from `.env.local` at the harness root (fall back to
the `TARGET_REPO_URL` environment variable if the file has none set). If
neither is set, stop and tell the user to run `harness init-repo <url>`
first — this skill writes labelled issues, and the label vocabulary only
exists on a repo `init-repo` has touched.

Resolve it to `owner/name` the same way `harness init-repo` does: strip a
`.git` suffix, strip a scheme/host (`https://github.com/`, `git@github.com:`,
`ssh://git@github.com/`), and the bare `owner/name` shorthand is already in
that form. Confirm the repo actually carries the label `harness:roadmap`
(`gh label list --repo <owner>/<name>`) before writing anything — its
absence means `init-repo` hasn't run against this repo yet, and the fix is
that command, not creating the label yourself here.

**Check for near-duplicates first.** List open `harness:roadmap` issues
(`gh issue list --repo <owner>/<name> --label harness:roadmap --state open`)
and show them to the user before creating anything that looks like it
overlaps. Creating a second roadmap item for something already tracked is a
human call, not a default.

## Write the roadmap issues

One `harness:roadmap` issue per distinct initiative the grilling surfaced —
roadmap-sized (what `/planner` draws a milestone from), not task-sized. For
each:

```bash
gh issue create --repo <owner>/<name> --label "harness:roadmap" \
  --title "<short, specific title>" \
  --body "<body>"
```

The body states: the problem and who has it, the success criterion for this
specific item, its explicit boundaries (what it does not cover), and any
constraint or priority relative to other roadmap items that bears on this
one. Do not add `harness:ready` — that checkbox belongs to the human, never
to a skill, per the harness's own label vocabulary
(`crates/workflows/src/common/labels.rs`).

## Write the business digest

Separately from the issues — this is what `/planner` actually reads before
opening a milestone, kept short on purpose (it is a digest, not the ADRs):

Write `.llocal/grill/<owner>/<name>/business-digest.md` **at the harness
root** (never inside the target repo's own clone — it is harness-local
state, already gitignored via `/.llocal/`). A few bullets: the cross-cutting
constraints and priorities that should shape *every* task the loop plans,
not the per-item detail already in the issues. If the file already exists,
show the user the diff and ask before overwriting — this file is read by a
paid `/planner` run, and silently replacing earlier context it was written
from is not something to do without a word.

## When done

List what was created (issue numbers and titles, the digest path) and stop.
Nothing here opens a milestone or a task — that is `/planner`'s job, run
through the ordinary `harness` loop once a human checks `harness:ready` on a
roadmap item.
