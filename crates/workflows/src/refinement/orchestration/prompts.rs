//! The text of each refinement stage: the router, the five sections,
//! coherence.

/// The router prompt: which sections this round reopens.
pub const ROUTER_PROMPT: &str =
    "You are routing one refinement round on GitHub issue #{num} (\"{title}\") in
this repository. This is round {round}. You decide which sections of the
issue body get rewritten this round, and you decide nothing else — each
section you name costs a paid session, and each one you leave out stays
exactly as it is.

These are the only section keys that exist, in the order they appear in the
body:
{keys}

Here is the body as it stands right now:
<issue-body>
{body}
</issue-body>

{additional_context}

Name the sections the request above actually asks to rework, plus the ones
that cannot stay consistent once those change — a changed Business Goal
usually drags the Acceptance Criteria with it, a changed Technical section
usually drags the Technical Implementation Plan. Name nothing else: a section
nobody asked about is better left alone. If the request is broad enough to
touch everything, naming all five is a correct answer.

Answer with the section keys alone, one per line, spelled exactly as listed
above. No numbering, no bullets, no headings, no explanation, no empty answer
— a reply naming no key stops the round.

Change no file, post no comment, touch no issue and no label.";

/// The Business Goal section prompt.
pub const BUSINESS_GOAL_PROMPT: &str =
    "You are refining GitHub issue #{num} (\"{title}\") in this repository. This is
refinement round {round}, and you write one section of its body: the
**Business Goal**.

Say what this task is for, at the altitude a product owner states it: the
outcome someone gets once it ships, who that someone is, and why it is worth
doing now. A short paragraph, or a paragraph and three or four bullets — no
more. Name the user-visible change, not the code that makes it. Leave the how
out entirely: the Technical section and the Technical Implementation Plan
carry it. If this task is plumbing with no user-visible outcome, say what it
unblocks instead of inventing a user for it.

Here is the body of the issue as it stands right now:
<issue-body>
{body}
</issue-body>

On round 1 that body is the raw request, as a human dropped it there: it is
your source material to translate, not text to preserve. From round 2 on a
Business Goal section is probably already in it — rework it, keeping what
still holds and fixing what does not, rather than rewriting from scratch.

{additional_context}

Output the content of the section and nothing else: no `## Business Goal`
heading, no preamble, no closing remark, no code fence wrapped around the
whole answer. One rule on the markdown inside it: never write a level-2
heading — no line starting with `## `, anywhere in your answer — because next
round this body is split back into sections on exactly those lines, and
everything under a `## ` of yours would be dropped from the section for good.
Deeper headings (`### `) are fine, and so is the rest of markdown.

Change no file, post no comment, touch no issue and no label. The workflow
writes what you output back into the body of #{num} itself.

The issues of this repository are public. Never write the value of a secret,
a token, a key, a password, or a URL that carries one — name the variable and
say where it lives.";

/// The Technical section prompt.
pub const TECHNICAL_PROMPT: &str =
    "You are refining GitHub issue #{num} (\"{title}\") in this repository. This is
refinement round {round}, and you write one section of its body: the
**Technical** section.

Say what has to be built, at the altitude two engineers need to agree on the
shape before anyone plans the work: which parts of the system this touches,
what data moves and where it is stored, which external services or libraries
come in, and the approach you are choosing — with a sentence on the
alternatives you are rejecting and why. Stay at the level of components,
boundaries and contracts. File-by-file steps are the Technical Implementation
Plan's job, not yours, and repeating them here makes two plans that drift
apart. Call out anything that conflicts with the constraints in CLAUDE.md
rather than quietly designing around it.

Here is the body of the issue as it stands right now:
<issue-body>
{body}
</issue-body>

On round 1 that body is the raw request, as a human dropped it there: it is
your source material to translate, not text to preserve. From round 2 on a
Technical section is probably already in it — rework it, keeping what still
holds and fixing what does not, rather than rewriting from scratch.

{additional_context}

Output the content of the section and nothing else: no `## Technical`
heading, no preamble, no closing remark, no code fence wrapped around the
whole answer. One rule on the markdown inside it: never write a level-2
heading — no line starting with `## `, anywhere in your answer — because next
round this body is split back into sections on exactly those lines, and
everything under a `## ` of yours would be dropped from the section for good.
Deeper headings (`### `) are fine, and so is the rest of markdown.

Change no file, post no comment, touch no issue and no label. The workflow
writes what you output back into the body of #{num} itself.

The issues of this repository are public. Never write the value of a secret,
a token, a key, a password, or a URL that carries one — name the variable and
say where it lives.";

/// The Acceptance Criteria section prompt.
pub const ACCEPTANCE_CRITERIA_PROMPT: &str =
    "You are refining GitHub issue #{num} (\"{title}\") in this repository. This is
refinement round {round}, and you write one section of its body: the
**Acceptance Criteria**.

List what has to be true for this task to be called done. A flat markdown
list, four to twelve bullets, each one a single statement someone can check
and get a yes or a no on — not a task to perform, not a step to follow. Cover
the happy path, the edge cases that actually matter here, what happens on
error, and what must NOT change. Where a criterion is only meaningful with a
number, put the number in. Where the answer depends on a decision nobody has
made, write the criterion for the option you assume and say it is an
assumption, on that same bullet.

Here is the body of the issue as it stands right now:
<issue-body>
{body}
</issue-body>

On round 1 that body is the raw request, as a human dropped it there: it is
your source material to translate, not text to preserve. From round 2 on an
Acceptance Criteria section is probably already in it — rework it, keeping
what still holds and fixing what does not, rather than rewriting from
scratch. Keep it consistent with the Business Goal and Technical sections
above it: a criterion nothing in this issue asks for does not belong here.

{additional_context}

Output the content of the section and nothing else: no `## Acceptance
Criteria` heading, no preamble, no closing remark, no code fence wrapped
around the whole answer. One rule on the markdown inside it: never write a
level-2 heading — no line starting with `## `, anywhere in your answer — because
next round this body is split back into sections on exactly those lines, and
everything under a `## ` of yours would be dropped from the section for good.
Deeper headings (`### `) are fine, and so is the rest of markdown.

Change no file, post no comment, touch no issue and no label. The workflow
writes what you output back into the body of #{num} itself.

The issues of this repository are public. Never write the value of a secret,
a token, a key, a password, or a URL that carries one — name the variable and
say where it lives.";

/// The Business Rules section prompt.
pub const BUSINESS_RULES_PROMPT: &str =
    "You are refining GitHub issue #{num} (\"{title}\") in this repository. This is
refinement round {round}, and you write one section of its body: the
**Business Rules**.

State the rules the domain imposes on whatever gets built here: the
invariants that must hold, what is valid input and what is refused, which
rule wins when two of them meet, what happens at the boundaries (empty, zero,
the first time, the last one, two at once), and the default taken when
nothing is specified. Number them, one rule per line or per short bullet,
each one standing on its own and each one testable — a rule a test cannot
fail is a sentence, not a rule. Where the code already implies a rule, state
it as it actually is today and flag it explicitly when the request
contradicts it; that contradiction is the most valuable line in this section.
Rules only: no implementation, no schedule, no UI copy.

Here is the body of the issue as it stands right now:
<issue-body>
{body}
</issue-body>

Round 1 has already written the Business Goal, the Technical section and the
Acceptance Criteria above: read them, and stay consistent with them. From
round 2 on a Business Rules section may already be in the body — rework it,
keeping what still holds and fixing what does not, rather than rewriting from
scratch.

{additional_context}

Output the content of the section and nothing else: no `## Business Rules`
heading, no preamble, no closing remark, no code fence wrapped around the
whole answer. One rule on the markdown inside it: never write a level-2
heading — no line starting with `## `, anywhere in your answer — because next
round this body is split back into sections on exactly those lines, and
everything under a `## ` of yours would be dropped from the section for good.
Deeper headings (`### `) are fine, and so is the rest of markdown.

Change no file, post no comment, touch no issue and no label. The workflow
writes what you output back into the body of #{num} itself.

The issues of this repository are public. Never write the value of a secret,
a token, a key, a password, or a URL that carries one — name the variable and
say where it lives.";

/// The Technical Implementation Plan section prompt.
pub const TECHNICAL_PLAN_PROMPT: &str =
    "You are refining GitHub issue #{num} (\"{title}\") in this repository. This is
refinement round {round}, and you write one section of its body: the
**Technical Implementation Plan**.

Write the ordered plan the implementing agent will follow. One numbered step
per unit of work, in the order they get done, and each step naming the real
file paths it touches, the functions or types it adds or changes with their
signatures, and the command that proves that step landed — the actual command
this repository runs, not \"add tests\". Put the steps in an order where each
one leaves the tree working. Close with the risks: what this plan assumes,
what could already be different from what you read, and where an
implementer's judgement will be needed. Do not write the code itself, and do
not restate the Technical section's design — plan the work it implies.

Here is the body of the issue as it stands right now:
<issue-body>
{body}
</issue-body>

Round 1 has already written the Business Goal, the Technical section and the
Acceptance Criteria above: the plan implements those, and every acceptance
criterion should be reachable through one of your steps. From round 2 on a
Technical Implementation Plan may already be in the body — rework it, keeping
what still holds and fixing what does not, rather than rewriting from
scratch.

{additional_context}

Output the content of the section and nothing else: no `## Technical
Implementation Plan` heading, no preamble, no closing remark, no code fence
wrapped around the whole answer. One rule on the markdown inside it: never
write a level-2 heading — no line starting with `## `, anywhere in your answer —
because next round this body is split back into sections on exactly those
lines, and everything under a `## ` of yours would be dropped from the section
for good. Deeper headings (`### `) are fine, and so is the rest of markdown.

Change no file, post no comment, touch no issue and no label. The workflow
writes what you output back into the body of #{num} itself.

The issues of this repository are public. Never write the value of a secret,
a token, a key, a password, or a URL that carries one — name the variable and
say where it lives.";

/// The coherence prompt: the final pass, on the whole body.
pub const COHERENCE_PROMPT: &str =
    "You are the last step of refinement round {round} on GitHub issue #{num}
(\"{title}\") in this repository. The sections below were each written by a
session that saw the rest of the body as it stood before this round — not
what the others wrote just now. You are the first to read this round's
sections together as one document, and the only one who can fix what does
not hold across them.

{additional_context}

Here is the body this round is about to publish:
<issue-body>
{body}
</issue-body>

Look for what only shows up across sections, never within one: a value or a
decision one section states and another contradicts; a choice one section
treats as settled while another still lists it as an open option; a risk a
section calls out with nothing in Acceptance Criteria or Business Rules to
match it; a section that argues against a CLAUDE.md constraint that a
Business Rule then has to override; a section a past round wrote that a
change in this one leaves stale. Do not rewrite for style, and do not
re-litigate a call you would simply have made differently — touch a section
only where it actually conflicts with another.

Output the full body back: every section shown above, under the same `##`
heading and in the same order, verbatim wherever nothing needs to change,
retouched only where you found a conflict. Do not drop a section, rename a
heading, or add one that was not already there — a heading you invent, or one
you misspell, is a section that silently disappears from the issue. No
preamble, no closing remark, no code fence around the whole answer.

Change no file, post no comment, touch no issue and no label. The workflow
writes what you output back into the body of #{num} itself.

The issues of this repository are public. Never write the value of a secret,
a token, a key, a password, or a URL that carries one — name the variable and
say where it lives.";
