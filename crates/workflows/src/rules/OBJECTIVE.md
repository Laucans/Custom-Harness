# Objective

Turn the rules the human validated while designing the product into tests
the product must pass, before any of it is built.

**Trigger.** The product's creation was initialized, a rule was changed
afterwards, or a development found a rule's test it cannot meet and
contested it.

**What it does.** Writes the rules down as the project's contract, writes
for each rule the test that proves it — its criterion copied as it is —
checks every rule is covered, and lands them through a change of its own.
A second reader, who did not write them, checks each test proves its
criterion and nothing weaker, and merges. A contested test is settled: the
test is fixed when it misreads its rule, the rule changed when it cannot
hold, or the contest refused with the reason.

**Why it matters.** The code is written to make these tests pass, so a
test that does not prove its rule lets a product ship that breaks it, and
one that proves more blocks the work for nothing.

**What it deliberately does not do.** It never writes product code. It
never weakens a test to make a development pass. It never merges what its
second reader found unfaithful: that waits for the human.

**What "done" looks like.** Every rule has its tests on the integration
branch, the issue is closed, and development may start — or the issue
says plainly what is unfaithful and waits on the human.
