# Objective

Turn one already-approved, clearly-scoped piece of work into a tested,
merged code change — without a person driving each individual step.

**Trigger.** A task exists, is scoped narrowly enough to execute, and has
been marked ready for work.

**What it does.** Picks up exactly one task at a time, writes the code it
calls for, runs the checks that prove the result actually works, submits the
change for integration, and confirms it was accepted before considering the
task finished. If no task is ready, it says so and stops — it never invents
work to stay busy.

**Why it matters.** It delivers engineering throughput without needing a
person to watch every commit, while still requiring real proof — passing
checks, an accepted change — before anything is called done.

**What "done" looks like.** The task is closed because the change that
completes it was verified and accepted into the project — never closed on
say-so.
