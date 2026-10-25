# Objective

Find, in one go, every unit a task still lacks, once its sessions kept
discovering them one stop at a time.

**Trigger.** A task's sessions stopped on missing tasks two rounds in a row
(`discovered: 2` under its `## Decision`, `common::restudy::due`): a journey
stops at its first failure, which hides the next one.

**What it does.** One read-only session, on its lane's own checkout of the
milestone's branch, walks every rule the task names step by step — the
screen's composition, each Micro-UI's needs and actions against the
manifests, the fixtures, the running server probed for every read and
command — and answers the complete list of missing slices, chained by the
ids they need and build, and the rule tests to contest. The harness opens
them all at once through the one opener, chains them, and parks the task on
every one of them.

**Why it matters.** `ForgeDesMondes`' #219 stopped six times in one day,
each time on the next gap; each stop was a serial wait of one to two hours.
One walk that does not stop at a failure turns six waits into one.

**What it deliberately does not do.** It never edits a tracked file,
commits, pushes, opens an issue or a pull request: the harness opens what
it declares. It never contests a rule itself: the contests are a note for
the next code session. It never runs twice on one discovery: the streak
records it, at most twice a streak. Interrupted once it took the streak (a
restart, a failed gate, a GitHub error), it counts as run: the task resumes
as a plain task.

**What "done" looks like.** Every missing slice is open under the milestone,
chained on what it needs, the task waits on all of them, and its
`## Decision` says what was opened and what to contest — or nothing was
missing beyond the open tasks, and the marker says the re-study ran.
