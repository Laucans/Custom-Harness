---
name: technical-grill-with-docs
description: technical-grill-me, then writes ADRs, a glossary, and the technical digest /planner reads — all local to the harness, never committed to the target repo. Use when the user wants the architecture interrogation to leave a durable trail, not just shared understanding.
disable-model-invocation: true
---

Call the Skill tool with `technical-grill-me` first, unless this same
conversation already ran it and nothing has been added since — in that case,
use its result directly rather than re-grilling. Do not take the step below
until the user has confirmed the shared understanding.

This skill writes **no issue and nothing to the target repository**. Its
output is local to this harness checkout, for two different readers: a human
(the ADRs, the glossary) and `/planner` (the digest, and only the digest).

## Where everything goes

Resolve `TARGET_REPO_URL` the same way `business-grill-with-issues` does
(`.env.local`, falling back to the environment variable; stop and point at
`harness init-repo <url>` if neither is set) to get `owner/name`. Everything
below lives under `.llocal/grill/<owner>/<name>/` **at the harness root** —
never inside the target repo's clone, and never committed: `/.llocal/` is
gitignored in this repository for exactly this reason.

## Write the ADRs

One file per architectural decision the grilling settled, numbered in the
order decided: `.llocal/grill/<owner>/<name>/adr/NNNN-<short-title>.md`.
Each carries the shape an ADR normally does — context, the decision, the
alternatives considered and why they lost, the consequences — written for a
human who will read it later and was not in this conversation. If an `adr/`
directory already exists, number the new ones after the highest existing
one; do not renumber or overwrite what's there.

## Write the glossary

`.llocal/grill/<owner>/<name>/glossary.md`: every project-specific term the
session used in a sense a newcomer wouldn't guess, one definition each. If it
already exists, add to it and flag any entry whose definition this session's
decisions contradict — a silently stale glossary is worse than a short one.

## Write the technical digest

`.llocal/grill/<owner>/<name>/technical-digest.md` — short on purpose (a
paid `/planner` run reads it before every milestone it opens; it is a
digest, not the ADRs). A few bullets: the invariants a task must not break,
the module boundaries that constrain how work can be sliced, and open
technical risks `/planner` should turn into their own blocking task rather
than silently assume away. If it already exists, show the user the diff and
ask before overwriting, same reasoning as the business digest.

## When done

List what was written (ADR numbers and titles, whether the glossary grew,
the digest path) and stop.
