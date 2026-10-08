# Objective

Decide, without relying on anyone's impression, whether a unit of work is
actually finished — and if it is, fold it into the project's accumulating
base of completed work.

**Trigger.** Evaluated whenever a unit of work might be complete.

**What it does.** Checks two hard facts: every piece of work inside the
unit is actually closed, and the automated checks on the result are actually
passing. Only when both are true does it integrate the result. If either
isn't true yet, it does nothing and simply waits for the next check.

**Why it matters.** It removes the risk of a unit of work being declared
finished and integrated on the strength of an impression rather than a
verified fact. There is no partial or best-effort integration here — a unit
of work is either not ready yet, or fully folded in.

**What "done" looks like.** The unit of work's result is integrated into the
project's accumulating base, and that happened only because both facts were
independently confirmed true.
