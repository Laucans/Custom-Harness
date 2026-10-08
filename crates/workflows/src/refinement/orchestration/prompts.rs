//! The text of each refinement stage: the router, the five sections,
//! coherence, and the advice on the technical refinement.

/// The router prompt: which sections this round reopens.
pub const ROUTER_PROMPT: &str =
    "You are routing one refinement round on GitHub issue #{num} (\"{title}\") in
this repository. This is round {round}. You decide which sections of the
issue body get rewritten this round, and you decide nothing else — each
section you name costs a paid session, and each one you leave out stays
exactly as it is.

These are the only section keys this round can rewrite, in the order they
appear in the body:
{keys}

Here is the body as it stands right now:
<issue-body>
{body}
</issue-body>

{additional_context}

Name the sections the request above actually asks to rework, plus the ones
that cannot stay consistent once those change — {drags}. Name nothing else: a
section nobody asked about is better left alone. If the request is broad enough
to touch everything, naming every key is a correct answer.

The keys above are the only ones this round can touch. The body holds other
sections, written by the other half of the refinement or by the loop; naming
one of those changes nothing and wastes the round.

Answer with the section keys alone, one per line, spelled exactly as listed
above. No numbering, no bullets, no headings, no explanation, no empty answer
— a reply naming no key stops the round.

Change no file, post no comment, touch no issue and no label.";

/// The Business Goal section prompt.
pub const BUSINESS_GOAL_PROMPT: &str =
    "You are refining GitHub issue #{num} (\"{title}\") in this repository. This is
refinement round {round}, and you write one section of its body: the
**Business Goal**.

Say what this issue is for, at the altitude a product owner states it: the
outcome someone gets once it ships, who that someone is, and why it is worth
doing now. A short paragraph, or a paragraph and three or four bullets — no
more. Name the user-visible change, not the code that makes it. Leave the how
out entirely: the Technical section and the Technical Implementation Plan
carry it. If this issue is plumbing with no user-visible outcome, say what it
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

Output the section's content only: no `## ` heading of your own anywhere
(the body is split on those lines next round, and text under one of yours
would be lost), no preamble, no closing remark, no code fence around the
whole answer; `### ` and the rest of markdown are fine. Write dense: no
restating of the request or of other sections, no filler — a sentence that
changes nothing for the engineer who builds this is cut.

Change no file, post no comment, touch no issue and no label: the workflow
writes your output into the body of #{num}. The issues are public — never
write a secret, a token, a key, a password or a URL carrying one; name the
variable and where it lives.";

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

The body's `## Architecture` section says what this task is in the
agent-native architecture (`docs/ARCHITECTURE.md`): design exactly that unit
and nothing beside it. A Capability reads through persisted queries only and
never imports, invokes or references another Capability or anything of the
write side; a DataCapability declares its effect and the fields it touches
and lives behind the DataGuard; a Concept is a definition with conformance
fixtures; a Micro-UI declares what it needs and the actions it may trigger.
Name the manifest or contract file the unit produces under `contracts/`, and
the gate in `.github/workflows/gates.yml` that will check it. The
repository's rules (the architecture block of CLAUDE.md, `docs/ARCHITECTURE.md`)
bind the design even where the existing code predates them: the unit goes
where the layout says, in the language the stack says — Rust for anything but
a Micro-UI, compiled to WebAssembly when a TypeScript host must load it.
Designing it in the host's language because that is what exists today is a
violation, not a choice: write no waiver, no fallback, no question for the
human — the human decided when installing the rules, and the toolchain's
cost belongs to the first Rust unit's plan. The declared unit is what it is:
a `capability` is a Capability crate with its manifest, never a \"local
implementation\" of the Concept in the host. State what the bridge costs,
then design it.

Here is the body of the issue as it stands right now:
<issue-body>
{body}
</issue-body>

The business sections (Business Goal, Acceptance Criteria, Business Rules) are
already in the body: they are the contract this design must satisfy, and you
do not rewrite them. Read the code the repository map points at — this is the
half of the refinement that needs it. From round 2 on a Technical section is
probably already in it — rework it, keeping what still
holds and fixing what does not, rather than rewriting from scratch.

{additional_context}

Output the section's content only: no `## ` heading of your own anywhere
(the body is split on those lines next round, and text under one of yours
would be lost), no preamble, no closing remark, no code fence around the
whole answer; `### ` and the rest of markdown are fine. Write dense: no
restating of the request or of other sections, no filler — a sentence that
changes nothing for the engineer who builds this is cut.

Change no file, post no comment, touch no issue and no label: the workflow
writes your output into the body of #{num}. The issues are public — never
write a secret, a token, a key, a password or a URL carrying one; name the
variable and where it lives.";

/// The Acceptance Criteria section prompt.
pub const ACCEPTANCE_CRITERIA_PROMPT: &str =
    "You are refining GitHub issue #{num} (\"{title}\") in this repository. This is
refinement round {round}, and you write one section of its body: the
**Acceptance Criteria**.

List what has to be true for this issue to be called done. A flat markdown
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
scratch. Keep it consistent with the Business Goal above it, and with any
Technical section already in the body: a criterion nothing in this issue asks
for does not belong here. This round has no access to the code — state
behaviour a person can observe, not how it is built.

{additional_context}

Output the section's content only: no `## ` heading of your own anywhere
(the body is split on those lines next round, and text under one of yours
would be lost), no preamble, no closing remark, no code fence around the
whole answer; `### ` and the rest of markdown are fine. Write dense: no
restating of the request or of other sections, no filler — a sentence that
changes nothing for the engineer who builds this is cut.

Change no file, post no comment, touch no issue and no label: the workflow
writes your output into the body of #{num}. The issues are public — never
write a secret, a token, a key, a password or a URL carrying one; name the
variable and where it lives.";

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

The Business Goal and the Acceptance Criteria were written just before this
section: read them, and stay consistent with them. From round 2 on a
Business Rules section may already be in the body — rework it,
keeping what still holds and fixing what does not, rather than rewriting from
scratch.

{additional_context}

Output the section's content only: no `## ` heading of your own anywhere
(the body is split on those lines next round, and text under one of yours
would be lost), no preamble, no closing remark, no code fence around the
whole answer; `### ` and the rest of markdown are fine. Write dense: no
restating of the request or of other sections, no filler — a sentence that
changes nothing for the engineer who builds this is cut.

Change no file, post no comment, touch no issue and no label: the workflow
writes your output into the body of #{num}. The issues are public — never
write a secret, a token, a key, a password or a URL carrying one; name the
variable and where it lives.";

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
not restate the Technical section's design — plan the work it implies. Every
unit of the agent-native architecture (`## Architecture` in the body) lands
with its manifest or contract and the gate that validates it: one step of the
plan writes that file, and the command that proves the step is the gate's own
check. The plan follows the stack and the layout of the rules, not the
language of the surrounding code: a TypeScript host that must load a Rust
unit gets a step for the Cargo workspace if none exists, one for the
WebAssembly build and one for its loading. Plan the rule-conforming design
only — never a fallback the human did not ask for.

Here is the body of the issue as it stands right now:
<issue-body>
{body}
</issue-body>

The business sections and the Technical section are already in the body: the
plan implements them, and every acceptance criterion should be reachable
through one of your steps. From round 2 on a
Technical Implementation Plan may already be in the body — rework it, keeping
what still holds and fixing what does not, rather than rewriting from
scratch.

{additional_context}

Output the section's content only: no `## ` heading of your own anywhere
(the body is split on those lines next round, and text under one of yours
would be lost), no preamble, no closing remark, no code fence around the
whole answer; `### ` and the rest of markdown are fine. Write dense: no
restating of the request or of other sections, no filler — a sentence that
changes nothing for the engineer who builds this is cut.

Change no file, post no comment, touch no issue and no label: the workflow
writes your output into the body of #{num}. The issues are public — never
write a secret, a token, a key, a password or a URL carrying one; name the
variable and where it lives.";

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

The issues are public — never write a secret, a token, a key, a password or
a URL carrying one; name the variable and where it lives.";

/// The advice prompt: should a human take part in the technical refinement?
pub const ADVICE_PROMPT: &str =
    "You have just seen the business half of the refinement of GitHub issue
#{num} (\"{title}\") in this repository. The technical half comes next: it
writes the Technical section and the Technical Implementation Plan, reading the
code. It can run two ways — unattended, as a stage of the development loop, or
asked for on the issue itself (label `harness:tech-refinement`), where a human
can read it and answer before anything is built.

Here is the body as it stands after this round:
<issue-body>
{body}
</issue-body>

Score how much this issue needs the technical half run **on the issue, under a
human's eyes**, before anything is built. The owner reads your score to decide
where to spend their attention across a whole board, so a 5 must mean something
a 3 does not:

5 — a wrong call here is expensive or irreversible: a paid service, a
    credential, a schema or public contract later tasks are built on.
4 — two real designs lead to different product behaviour, or a business rule
    looks impossible or expensive against the code as it is.
3 — one decision is open and the body does not settle it, but any of the
    plausible answers can be changed later at a normal cost.
2 — the approach follows from the code, with a detail or two left to judgement.
1 — mechanical: the repository already shows how, and any engineer reading it
    would make the same calls.

Answer in this exact shape, and nothing else:
Line 1: `technical-refinement: N/5`, N being that score.
Line 2: `human-in-the-loop: yes` or `human-in-the-loop: no` — yes from 4 up,
and whenever a question below has no answer anywhere in the body. The two lines
must agree.
Then two to five short bullets: the reasons, and — for `yes` — the precise
questions the human has to answer.

Score the issue in front of you, not the average issue: if nothing here is
genuinely open, 1 is the useful answer and inflating it costs the owner a
reading for nothing.

Change no file, post no comment, touch no issue and no label. The workflow
posts your answer as a comment on #{num} itself.

The issues are public — never write a secret, a token, a key, a password or
a URL carrying one; name the variable and where it lives.";
