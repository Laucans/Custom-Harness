# Objective

Catch problems in a proposed code change before it is trusted, by examining
it adversarially rather than taking it at face value.

**Trigger.** A proposed code change has been explicitly marked as waiting
for a review.

**What it does.** Examines the change on its own merits — what it claims to
do versus what it actually does, what it may have missed, what could break
— and writes up its findings for a human to act on.

**Why it matters.** A second, skeptical look catches what the person who
made the change is least likely to notice themselves, before it affects
anyone relying on the project.

**What it deliberately does not do.** It does not decide the outcome. It
produces a written opinion; a human still decides what happens to the
change.

**What "done" looks like.** The proposed change has a written review
attached, naming every concern found or explicitly stating that none were.
