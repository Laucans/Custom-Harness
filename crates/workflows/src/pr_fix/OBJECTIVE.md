# Objective

Give a proposed change whose automated checks have broken one honest attempt
at being fixed, instead of leaving it parked until a person has time.

**Trigger.** Someone has marked a proposed change as worth repairing, and at
least one of its automated checks has actually failed — not merely still be
running.

**What it does.** Reads what broke and anything already said about it,
diagnoses the cause from the real output rather than the failure's name,
makes the change needed, confirms the project's own checks pass, and submits
the correction onto the same proposed change.

**Why it matters.** A broken check blocks everything behind it, and the
person who could fix it is usually elsewhere. One attempt, made promptly, is
often the difference between a change that lands today and one that waits a
week.

**What it deliberately does not do.** It never makes the failure go away by
weakening or removing the check that caught it — a check that is wrong is
reported, not edited. It stays inside the scope of the change it was called
on. It does not decide the repair worked, and it does not accept the change
into the project; both are read or decided elsewhere, afterwards. And when
the cause genuinely needs a person — a decision, an access it does not have,
a problem living somewhere else — it says so and changes nothing.

**One request, one attempt.** The request is consumed when the attempt
begins, not when it succeeds, so a change nobody can repair cannot quietly
consume effort over and over. Asking again is a deliberate act.

**What "done" looks like.** Either a correction has been submitted and the
checks will say whether it worked, or there is a plain statement of what is
wrong and what it would take — never a silent pass.
