# Objective

Pay back, one file at a time, the tests the code owes: the places where the
code can be broken on purpose and every test still passes.

**Trigger.** The project's continuous integration judged a few files by
breaking them on purpose, found changes no test noticed, and filed them as
debt against the file — or found that debt still owed and asked again.

**What it does.** Reads which file is owed tests and what was found, checks
what is still uncaught today, writes the tests that notice each break,
proves each one does, runs the project's own checks, and lands the tests
through a reviewed change of their own.

**Why it matters.** A test suite that passes says nothing about the code it
never looks at. Tests rot quietly — weakened, deleted, outgrown by the code —
and a debt nobody pays grows until a real break ships unnoticed.

**What it deliberately does not do.** It never changes the code under test
to make a finding disappear. It never removes, skips or weakens a test or a
check. A break with no observable effect is recorded as such, with its
reason, rather than tested. A real bug a new test reveals is reported, not
fixed here. And it never lands a change whose checks are red.

**One request, one attempt.** The request is consumed when the attempt
begins, not when it succeeds, so a file nobody can test does not quietly
consume effort over and over. The next judgement of that file asks again
if the debt is still owed.

**What "done" looks like.** Either the tests are landed and the debt closed,
or the debt was already paid and is closed saying so, or there is a plain
statement of what is still uncaught and why — never a silent pass.
