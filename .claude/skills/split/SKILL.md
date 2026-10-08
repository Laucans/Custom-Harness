---
name: split
description: Turn one harness:ready milestone into the tasks that deliver it — harness:agent/harness:human issues chained with blocked_by, each carrying a branch: line, each scoped for one fresh session and exactly one SPEC. Writes no document. Use when a milestone is marked ready to be worked, or when the user says "/split" or "you are split".
---

# Role: Split

You take **one** `harness:ready` milestone issue and open every task that
delivers it, in order, each `blocked_by` the one before it.

This is the human-invoked path. The harness also runs this same
decomposition on its own, automatically, whenever a milestone carries
`harness:ready` — see `crates/workflows/src/split/` if you want to read
that path's own prompt. Run this skill by hand when you want to review the
decomposition before it writes anything, or to split a milestone the
harness hasn't gotten to yet.

**Planning further out than one milestone is not this skill's job.**
Breaking a large goal into milestones happens one level up — see
`/planner` (or the harness's own equivalent) if that's what's missing. This
skill starts from a milestone that already exists and is already
`harness:ready`; it stops at tasks.

You write **no file.** Task tracking lives in GitHub issues — there is no
task document. You do **not** write code, write a SPEC, install
dependencies, or create accounts. Read and plan in plan mode if it's
available; leave it before section 4, which writes to GitHub.

**Issues on a public repo are public.** Name a credential (the key, never
its value), and keep dashboard and project URLs out of every body you
write.

## 1. Read first

- The project's own documentation, if the repo carries any (a `CLAUDE.md`,
  a project doc, an architecture doc) — a task must fit how the project is
  actually built, not drift from it.
- The milestone issue itself: `gh issue view <m>`.
- Its existing tasks, if any —
  `gh api repos/{owner}/{repo}/issues/<m>/sub_issues --jq '.[]|"\(.number) \(.state) \(.title)"'`.
  Slice only what's still missing; never reopen one that's already there.

## 2. Verify the ground truth before slicing

**Check the repo, not the milestone text.** A milestone's own wording
describes what was planned; it is not evidence of what's actually built.
Before slicing on top of an earlier task, confirm its output actually
exists.

Report any discrepancy before continuing. If the milestone assumes a
foundation that isn't really there, say so and stop — slicing on top of it
wastes the whole decomposition.

## 3. Interview before writing

Use `AskUserQuestion`, but only for real forks in the road — the project's
own documentation and the milestone body should already settle most of
this level:

- **Slice boundaries** — where exactly one slice's scope ends and the next
  one's begins, when it isn't obvious from the milestone body.
- **Sequencing** — which slice must land first for the rest to build on it
  (a schema before the code that reads it, an API before the UI calling
  it).
- **`needs_human`** — flag true only for account creation, an interactive
  login, a payment decision, or anything else only a human in a browser can
  do; ask when it's genuinely unclear which side of that line a slice falls
  on.

## 4. Open the tasks

`gh` has no native flag for sub-issues or for dependencies, so the links go
through `gh api`. Create the parent link and the blocker before what
depends on it — a link needs both issues to exist. If the milestone has no
branch of its own yet, create it before the first task:

```sh
# 0. the milestone's own branch, once, off the integration branch it's
#    actually built on (ask if that isn't obvious)
gh api repos/{owner}/{repo}/git/ref/heads/<base> --jq .object.sha
gh api -X POST repos/{owner}/{repo}/git/refs \
  -f ref="refs/heads/milestone/<m>-<slug>" -f sha=<sha from above>
# 1. one task, under the milestone
gh issue create --label harness:agent --title "<task>" \
  --body-file <file>                      # harness:human instead, if needs_human
# 2. the internal id a link needs — NOT the issue number
gh api repos/{owner}/{repo}/issues/<n> --jq .id
# 3. parent it under the milestone
gh api -X POST repos/{owner}/{repo}/issues/<m>/sub_issues \
  -F sub_issue_id=<id of the task>
# 4. block it on each task it builds on — not on "the previous one"
gh api -X POST repos/{owner}/{repo}/issues/<n>/dependencies/blocked_by \
  -F issue_id=<id of a task it depends on>
# 5. its side of the architecture
gh issue edit <n> --add-label harness:read-side   # or harness:write-side
```

Long bodies go through `--body-file`, never inline: a body typed on the
command line gets mangled by the shell.

**The task body** has two sections, both written once and never rewritten
by a refinement. `## Scope` opens with a `branch: <type>/<slug>` line
(`feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `AIchore` — per
`.claude/skills/commit/SKILL.md`), a blank line, then a brief: what this
slice covers and what it explicitly does not — the next slice's territory,
not an oversight. `## Architecture` places the slice in the agent-native
architecture (`docs/ARCHITECTURE.md`), as `key: value` lines:

```markdown
## Architecture

unit: capability          # capability | micro-ui | concept | data-capability
                          # | persisted-query | composition | invariant
                          # | migration | infrastructure
system: credit            # the bounded context
concept: Risk@3           # the versioned Concept it implements, if any
effect: insert            # data-capability only: insert | update | delete | upsert
touches: -                # data-capability only: existing fields it changes
side: harness:read-side   # or harness:write-side — see below
```

This is **not** the SPEC: `/business-analyst` writes that, in its own pass,
once the task exists and is picked up.

**One slice, one unit.** A slice that would build two units (a Capability
and the Micro-UI that reads it) is two slices. The second depends on the
first.

**Order so something testable exists early**, and make the last slice the
one that proves the whole milestone actually runs end to end — not a
cleanup task tacked on after everything else.

### Label the side, and chain only what really depends

Every task carries `harness:read-side` or `harness:write-side`, next to
`harness:agent`/`harness:human`:

- **read side** — capability, micro-ui, concept, persisted-query,
  composition, and a data-capability whose effect is `insert` with nothing
  in `touches`. These share nothing by design: chain one `blocked_by`
  another **only** when it reads what the other produces. Unchained
  read-side tasks run **in parallel**, each in its own session and clone.
- **write side** — a data-capability that updates, deletes or upserts (or
  touches existing fields), an invariant, a migration, infrastructure.
  Chain each write-side task onto the previous write-side one (mutations are
  serialized, as behind the DataGuard), and put them before the readers that
  build on them. A human merges every write-side PR; the loop waits.

### Mark `harness:human` honestly

`harness:agent` is the default. `harness:human` is for the slice's content
— account creation, an interactive login, a payment decision — never for
"this one feels risky" or "I'd rather a person checked this first." A
human task still gets a brief; it simply never runs through `/code`.

### Close the tap you used

Once every task is open (or you've decided none is needed), remove
`harness:ready` from the milestone and add `harness:triggered` — in that
order, last:

```sh
gh issue edit <m> --remove-label harness:ready --add-label harness:triggered
```

This marks the milestone as already split, so neither you nor the harness
splits it a second time. Do this even when the plan opened nothing — a
milestone that genuinely needs no further task has still been looked at.

## 5. Hand off

End by telling the user:

> Tasks #a → #z opened under milestone #M, each carrying a `branch:` line,
> its `## Architecture` section and its side; `blocked_by` links only where
> one really builds on another. `harness:ready` removed, `harness:triggered`
> added — the loop (or `/business-analyst`, by hand) picks the runnable ones
> up from here, several at a time on the read side.

Do not roll straight into writing a SPEC for the first task — that is
`/business-analyst`'s job, in a clean context.
