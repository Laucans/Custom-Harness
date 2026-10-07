# Objective

Make a repository ready to be worked by the system — once, before anything
else runs against it.

**Trigger.** A repository is being brought under the system's management for
the first time (or an operator asks to re-check one that already is).

**What it does.** Establishes the shared vocabulary the rest of the process
depends on to recognize what kind of work an item represents and what state
it is in, and sets up the line where in-progress work accumulates before it
is fully finished. It also checks the repository's current state against
what is required and reports any gap honestly, rather than assuming
everything is in order.

**Why it matters.** Every later decision the system makes rests on this
shared vocabulary and starting point existing correctly. Getting it wrong
once would make everything built on top of it unreliable. Running it again
on a repository that is already set up changes nothing that doesn't need
changing.

**What "done" looks like.** The repository carries the vocabulary and the
starting point the rest of the process needs, and any remaining gap has been
surfaced in plain terms rather than left to be discovered later.
