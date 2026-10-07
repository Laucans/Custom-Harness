---
name: grill-to-roadmap
description: Turns a grilling session into exactly one harness:roadmap issue on the target repository — the major version the session converged on — plus one grill:backlog issue for everything deferred and the business digest planner reads. Reads the business grilling as its mandatory input and the technical one as an optional one. Use when the user wants the interrogation to produce tracked work, not just shared understanding.
disable-model-invocation: true
---

Call the Skill tool with `business-grill-me` first, unless this same
conversation already ran it and nothing has been added since — in that case,
use its result directly rather than re-grilling. Do not take the step below
until the user has confirmed the shared understanding: `business-grill-me`
itself says so, and writing to GitHub on an unconfirmed understanding is
worse than writing nothing.

## The technical grilling is an input, never a prerequisite

The roadmap item is a product decision, so the business grilling is what
this skill cannot do without. The architecture grilling sharpens two parts
of the body — the constraints that bear on how the item gets built, and the
open technical risks worth naming as boundaries — so use it when it exists,
in this order of preference:

1. **This same conversation**, if `technical-grill-me` already ran in it.
2. **`.llocal/grill/<owner>/<name>/technical-digest.md`**, if a previous
   session's `technical-grill-with-docs` wrote one. Read the ADRs next to it
   (`adr/`) only if the digest leaves a constraint ambiguous.
3. **Neither** — write the roadmap item anyway, and say in your closing
   summary that the technical input was absent, so the user can decide
   whether to grill the architecture and revisit the body.

Never re-grill the architecture yourself here, and never write to
`technical-digest.md`, the ADRs, or the glossary: those belong to
`technical-grill-with-docs`, and two skills editing one file is how a digest
silently loses a session's decisions.

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

**One roadmap item is open at a time.** List the open ones first
(`gh issue list --repo <owner>/<name> --label harness:roadmap --state open`).
If one already exists, stop and show it: the next roadmap item is born of the
next grilling, once this one is delivered. Opening a second in parallel is a
human call, never a default.

## Write **one** roadmap issue

**One session of grilling produces exactly one `harness:roadmap` issue.** It
is the major version this grilling converged on — what the session *retained*
and did not defer. Everything else goes to the backlog below.

This is the sizing mistake worth naming, because it is the one that actually
happened: eleven roadmap issues out of one session, each of them really a
**milestone**. A roadmap item is not "a distinct initiative that came up"; it
is the whole of what the product should become next, the thing a sequence of
milestones delivers. If what you are about to write could be delivered by one
coherent batch of work, it is a milestone, and it is not yours to open —
`planner` draws those from the roadmap item once a human checks
`harness:ready`.

The test, applied out loud before writing: *would delivering only this leave
the product in a state worth having?* If the answer needs two of your
candidates, they are one roadmap item. If a candidate's absence would not be
noticed, it is backlog.

```bash
gh issue create --repo <owner>/<name> --label "harness:roadmap" \
  --title "<short, specific title>" \
  --body-file <file>
```

### The shape of the body

A roadmap item is the **product vision**. Its first job is to make the product
*understandable* — a reader who knows nothing must finish the body able to say
what the thing is, who uses it, and what they do with it. Only then does the
item say where it is going and in what order. The milestones `planner` draws
from it are where each feature gets defined precisely; the roadmap is where
the product gets explained.

The test that matters more than any length rule: **hand the body to someone
who was not in the grilling. Can they describe the product back to you?** If
they can only recite what it will not do and what constrains it, the item has
failed, however correct every section is.

Write these sections, in this order, with these headings — translated into
the language the grilling was conducted in, since the only readers are the
people who were in it:

```markdown
## The vision
One paragraph. What the product is, for whom, and what changes in their life
once it exists. Not a feature list — the sentence the user would say.

## The product
**The heart of the item, and the section to write first.** Describe the thing
itself, in two passes:

- *Les objets* — the nouns the product manipulates and what each one is: the
  vault is canon, a session is the unit of prep and push, an Adventure
  document is what Foundry swallows. A reader who does not know the
  vocabulary cannot understand anything downstream, and these nouns are what
  milestones will be named after.
- *Une séance type* — the user's walkthrough, in order, in plain prose: what
  they open, what they paste, what the product shows them, what they approve,
  what they get. Concrete and narrated, not a capability list.

This section has no length cap. It is the one part of the body that may not
be compressed, because everything else is unreadable without it.

## Starting point
Who has the problem, how they cope today, and what the repo actually holds —
checkable, not an impression. Three lines.
("the repo holds a README and nothing else", "4h of prep for 2h30 of play")

## The path
Three to seven ordered steps. Each one is a **state** — what is true once it
is done, never a task — stated as a bold line, then two or three lines that
say what the product can do at that point, what it still cannot, and why this
step has to come before the next:

  **2. An NPC created by hand reaches Foundry playable.** The vault can be
  read and written, one entity exists end to end, and the export path is
  proven on real content. Nothing is extracted from a narrative yet — the
  point is that the last mile works before anything upstream feeds it.

A bare one-line step is the same failure as a body with no product section: a
reader sees a label and cannot tell what it buys. Close the section with the
boundary, in these words or near them: *the order is a constraint; the
slicing belongs to `planner`.* Without that line a reader takes the steps for
milestones.

## Success criterion
One scenario, end to end, observable. Then the "if only one thing works: …".
When the product section already narrates the walkthrough, do not retell it —
point at it and state the gate.

## Technical implementation choices
The decisions already made about **how** the product is built, each in one
line, and only the ones that bound everything downstream: the product's form
and the stack it implies, the integration points imposed by the outside world,
the boundary that keeps a form choice from becoming a dead end, the data
shapes a model can actually produce.

What belongs here is **decided and structural**. A feature's field-by-field
design does not — that is the SPEC's ground — and neither does a choice nobody
has made yet: an undecided one goes to Open risks, named as undecided, never
written here as though it were settled.

If a technical grilling has run, this section is where its decisions land,
stripped of their reasoning. If none has, say so in one line and fill the
section from what the business grilling already settled — a reader must not
mistake "nobody decided yet" for "nothing constrains it".

## Boundaries
One line per exclusion, plus its reason in four words. The detail lives in
the linked `grill:backlog` issue — referenced, never copied.

## Non-negotiable constraints
Seven bullets at most. The **decision**, never its justification: that
belongs in an ADR under `.llocal/grill/<owner>/<name>/adr/`.

## Open risks
What is undecided and blocks. One line each, so `planner` opens a blocking
task instead of discovering it mid-milestone.
```

Four rules hold the format together:

- **The product section is the one that may not be cut.** Everything after it
  is only meaningful once a reader knows what the thing is. If the body has to
  shrink, shrink the constraints, the boundaries, the risks — never this.
- **Describe, do not enumerate.** The product is understood through its nouns
  and a walkthrough, not through a list of capabilities. "The GM pastes the
  session's account into a note and the side panel fills with the NPCs it
  found" teaches the product; "NPC detection, vault cross-check, draft
  generation" teaches nothing to someone who does not already know it.
- **One fact, one place.** The roadmap carries the *decision*; the ADR
  carries *why*; `grill:backlog` carries what was *deferred*; the digests
  carry what `/planner` *reads*. The failure mode is all four restating each
  other, and then disagreeing after one is edited.
- **Structural technical choices in, feature design out.** The form of the
  product, the stack it implies, the integration points, the boundaries that
  keep a choice from becoming a dead end — those belong in the body, because
  every milestone inherits them. Which crate owns what, and how one feature
  behaves field by field, do not: that is `planner`'s, `split`'s and the
  SPEC's ground.

The path is what keeps the vision from being a wish: steps are states, because
a state can be checked off, and a task cannot be sliced by `planner` without
being re-decided.

**On length.** There is no word cap. The budget that matters is proportional:
if the sections describing the product are shorter than the sections
constraining it, the item is upside down. That inversion is the measurable
symptom of the failure this format exists to prevent.

The technical input earns its place in four sections: the implementation
choices, the constraints, the ordering those constraints force on the path,
and the open risks. Take the *decision* from it and leave the reasoning where
it was written.

Two anti-patterns, both named because both actually happened on the same
item:

- **Three documents at once** — a roadmap, a spec and a set of ADRs, fourteen
  constraints deep, half of them justifications, with no path anywhere in it.
- **A body made of scaffolding only.** Destination, boundaries, constraints,
  risks, every section correct — and nowhere does it say what the product *is*.
  The second rewrite fixed the structure and left the reader just as lost,
  which is how this version of the format came to exist. Scaffolding is what
  surrounds the product; it is not a description of it.

### If the repo has no CI, that is step 1 of The path

Check before writing the body:

```sh
gh api repos/{owner}/{repo}/contents/.github/workflows --jq '.[].name'
```

A 404, or no workflow that runs the project's tests, means **step 1 of The
path is CI** — not a section of its own, and not a paragraph quoted into the
body. One line, in the form every other step takes:

> 1. CI is green on `main_agent` and `milestone/**`: a workflow at
>    `.github/workflows/ci.yml` runs the project's own gates on `push` and
>    `pull_request`. Nothing downstream is verifiable before it.

Why it comes first rather than later: nothing downstream of it can be trusted
without it. A PR's checks are what decides whether a task's work merges, and
a repository with no checks at all reads as **not green** by construction —
so a milestone would wait forever on a signal that never arrives.

The two branch patterns are not decoration. `main_agent` is where finished
milestones land, and `milestone/**` is where each task's PR targets — a
workflow naming only the default branch leaves every task PR unchecked.

Long bodies go through `--body-file`, never inline: a body typed on the
command line gets mangled by the shell.

Do not add `harness:ready` — that checkbox belongs to the human, never to a
skill, per the harness's own label vocabulary
(`crates/workflows/src/common/labels.rs`).

## Write the deferred rest to the backlog issue

Everything the grilling surfaced and set aside goes to **one** issue labelled
`grill:backlog` — not one issue each, and never a `harness:roadmap` issue.
That label lives outside the `harness:` namespace on purpose: the loop and
the router never see it, so nothing here can be mistaken for work to plan.
`harness init-repo` creates it; confirm it exists the same way you confirmed
`harness:roadmap`.

**Update it, do not recreate it.** It accumulates across sessions:

```bash
# is there one already?
gh issue list --repo <owner>/<name> --label "grill:backlog" --state open
# then either append to its body (read it, add a dated section, write it back)
gh issue edit <n> --repo <owner>/<name> --body-file <file>
# or, only if none exists:
gh issue create --repo <owner>/<name> --label "grill:backlog" \
  --title "Backlog — ideas deferred during grilling" --body-file <file>
```

Append a section dated with today's date, one line per deferred idea and
**why** it was deferred — "needs the territory tree first", "nobody asked for
it yet", "too expensive for the value". The reason is what makes the line
useful six months on; a bare list of features is a wish list nobody reads.
Never delete an earlier session's section.

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

List the roadmap issue, the backlog issue (created or updated, with what you
appended), and the digest path. Then stop.

Nothing here opens a milestone or a task — that is `planner`'s job, run
through the ordinary `harness watch` loop once a human checks `harness:ready`
on the roadmap item. Say that last part out loud: nothing moves until that
gesture.
