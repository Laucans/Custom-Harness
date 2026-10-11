# Objective

Settle, without a human where an agent may, a question a task's refinement
raised about its place in the milestone's graph — and hand the rest to a
human with the analysis already done.

**Trigger.** A task carries `harness:needs-ai-decision`: its technical
refinement found the design builds on an id its `needs` line does not
name, and wrote the question under `## Decision`.

**What it does.** First mechanically — an id a sibling task produces, or
one the repository already holds, is chained without a session. Then, for
what is left, one short session over the milestone's index (every task's
`## Architecture`, never a refined SPEC) names which task produces each id.
What it settles, it writes: the task's `needs`, its `blocked_by` links, a
sibling's `produces` when the id was nobody's yet, and the verdict under
`## Decision`.

**Why it matters.** Twenty tasks refined in parallel cannot each rebuild
the milestone's dependency tree, and a human asked twenty graph questions
answers none of them well. One writer of the graph at a time, with a human
only where the plan itself is in question, keeps the board coherent and
the human's attention for what is theirs.

**What it deliberately does not do.** It never changes what a task
`produces`, never cuts a task, never touches a task that has started, and
never pays a second session to disagree with a first: all of these go to a
human under `harness:needs-decision`, with its reasoning in a comment.

**What "done" looks like.** The task's `## Architecture` names every id
it builds on and waits on every task that produces one — or a human knows
exactly which decision is theirs.
