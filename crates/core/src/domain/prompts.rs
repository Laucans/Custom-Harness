//! The preamble, the scope block, and how they compose.
//!
//! What **every** session receives, whatever workflow launches it: the
//! EXECUTION CONTEXT block — where it is stated that you can ask no questions,
//! how to stop, and which branch you are working on — and the SCOPE block,
//! which carries the milestone and the issue. Instructions specific to a stage
//! are added to that and are not here.
//!
//! **This module knows no workflow.** A stage's prose is passed to it as an
//! argument.
//!
//! The preamble requires `AGENT_LOOP_OK:` at the end of the response and
//! [`crate::domain::markers`] reads it back: two halves of the same contract,
//! and a test requires them to name the same string.
//!
//! # A single substitution, one pass
//!
//! Python had two functions — `fill`, sequential, and `splice`, one pass — and
//! applied `splice` only to refinement. But the SCOPE block carries issue bodies
//! and titles, **which come from GitHub**: a `{body}` written into a milestone
//! body would be substituted by the next pass. That is exactly what `splice`
//! existed to prevent, applied to the wrong place. The two are merged here into
//! a single pass: identical when values are clean, safe when they are not.

/// The name cited in blocks when the caller provides none.
///
/// Deliberately generic: naming a specific workflow's CLI here would put an
/// instance into the vocabulary, which this layer has no right to carry.
pub const INJECTOR: &str = "the harness runner";

/// What an empty body says, in words.
///
/// Said rather than left blank: a stage reading a blank section cannot
/// distinguish "nothing was written" from "injection failed", and only one of
/// the two deserves a stop.
pub const EMPTY_BODY: &str = "(empty — nothing has been written into this issue yet)";

const PREAMBLE: &str = "
--- EXECUTION CONTEXT (injected by @INJECTOR@) ---
You are running head-less in an unattended loop (`claude -p`). Nobody will
read this output before the run ends and nobody can answer a question.
These rules override the skill's interactive stopping points:

1. Where the skill waits for a go-ahead or a confirmation (/code steps 2
   and 3), write your findings into your reply and carry on. Reporting
   stays mandatory; waiting does not.
2. Where the skill says to ask because a choice is genuinely ambiguous, do
   not guess. Stop, change nothing further, and end your reply with the one
   line `AGENT_LOOP_STOP: <one-line reason>`. The loop halts and a human
   picks it up. Stopping is a correct outcome, not a failure.
3. The integration branch for this run is `@BRANCH@`. Wherever a skill says
   `main` as the PR base or the branch-off point, read `@BRANCH@`: branch
   off it, and `gh pr create --base @BRANCH@`. The rest of CLAUDE.md's
   Repository etiquette stands unchanged — branch -> PR, merged the way this
   stage's own instructions say, and never a direct push.
4. Stage by name, never `git add -A`. The tree may carry unrelated
   in-flight work that is not yours to commit.
5. Do not start another pipeline stage as its own process, and do not
   /clear. The loop runs one process per stage. Where this prompt names a
   second skill to continue into, that continuation is part of this same
   stage — not a new one, and not something to hand off.
6. End your reply with `AGENT_LOOP_OK: <one-line summary>` if the stage
   completed, or `AGENT_LOOP_STOP: <reason>` if it did not.
--- END EXECUTION CONTEXT ---";

const SCOPE: &str = "--- SCOPE (injected by {injector}) ---
The issues below are the whole brief, from the widest to the narrowest. All
of them are GitHub issues: what you produce goes back into the issue, never
into a file under docs/.

Only ISSUE #{num} is yours to work on. The roadmap, the milestone and the
sibling tasks are there so you understand where it sits and what its
neighbours own — do not implement, plan, or rewrite any of them. If the
current issue seems to need something a sibling owns, assume that sibling
delivers it and stay within your own scope.
{roadmap_block}
MILESTONE #{milestone} — {milestone_title}
{milestone_body}

TASKS OF THIS MILESTONE (what each neighbour covers; none of them is yours\nexcept the current one)
{siblings}

CURRENT ISSUE #{num} — {title}
{body}
--- END SCOPE ---";

const TASK: &str = "--- SCOPE (injected by {injector}) ---
ISSUE #{num} below is the whole brief, and the whole of your work. It is a
GitHub issue: what you produce goes back into the issue, not into a file under
docs/.

It sits in milestone #{milestone} among sibling tasks that are not yours. They
are deliberately not reproduced here — this stage works against a spec that is
already written, and a neighbour's internals are not part of it. If you find
you do need one, read it rather than guess: `gh issue view <number>`. Do not
implement, plan or rewrite any of them.

CURRENT ISSUE #{num} — {title}
{body}
--- END SCOPE ---";

const STACK: &str = "--- REPOSITORY CONFIGURATION (injected by {injector}) ---
This checkout's own configuration, verbatim: how the project is built, tested
and linted. Read the scripts and runner settings from here instead of opening
these files again. Configuration only, never the source; if a command taken
from here fails, the file on disk wins — re-read it and carry on.

{files}
--- END REPOSITORY CONFIGURATION ---";

const SIGNATURES: &str = "--- PUBLIC SIGNATURES (injected by {injector}) ---
The public shape of the files this task names — what you can call, with what
arguments; bodies left out on purpose. Read it instead of opening these files
to learn their API. Only public items, only the files the issue names: a
neighbour not listed here is a file you still open. Outside TypeScript it is
derived by pattern, not by a compiler — where it disagrees with the code, the
code wins.

{files}
--- END PUBLIC SIGNATURES ---";

const HIERARCHY: &str = "--- HIERARCHY (injected by {injector}) ---
Where issue #{num} sits, for context only. Your work is issue #{num} alone:
the roadmap, the milestone and the sibling tasks are there so you understand
what its neighbours own — do not specify, plan or rewrite any of them. If
#{num} seems to need something a sibling owns, assume that sibling delivers
it and stay within #{num}.
{roadmap_block}
MILESTONE #{milestone} — {milestone_title}
{milestone_body}

TASKS OF THIS MILESTONE (what each neighbour covers)
{siblings}
--- END HIERARCHY ---";

/// An issue as a session must see it: its number, its title, its body.
///
/// Serializable because it enters the resume point: it is the scope that an
/// interrupted run must recover so that the following session does not start
/// blind.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Named {
    /// The number, as text — it only enters into prose.
    pub number: String,
    /// The title.
    pub title: String,
    /// The body. For a task, the body **is** the SPEC.
    pub body: String,
}

/// A task of the milestone, as its neighbours see it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sibling {
    /// The number, as text.
    pub number: String,
    /// The title.
    pub title: String,
    /// A few words: `done`, `open`, `waiting for merge`…
    pub status: String,
    /// What this neighbour is for, in its own words — a few lines, not its
    /// whole body.
    ///
    /// **Why not the whole body.** Measured on one finished milestone: the five
    /// task bodies came to ~39 000 tokens once refined, against ~850 while they
    /// were still raw slices. The expensive part is also the useless part — a
    /// neighbour's implementation plan and its assumptions are its internals,
    /// and injecting them invites a session to implement its neighbour's work
    /// or to "fix" an inconsistency that is not its own. What a session needs
    /// from a neighbour is the boundary: what it covers, what it does not.
    ///
    /// Empty is normal: a task nobody has written anything about yet.
    pub gist: String,
}

/// What a session works on: a task, in a milestone, in a roadmap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// The roadmap item the milestone came from, when it could be found.
    pub roadmap: Option<Named>,
    /// The tasks of the milestone, the current one included.
    pub siblings: Vec<Sibling>,
    /// The milestone.
    pub milestone: Named,
    /// The task.
    pub task: Named,
}

/// How much of the brief a stage's prompt carries.
///
/// The three are not degrees of the same block: each answers a different
/// question about the stage. A stage that opens a task has no brief at all; one
/// that writes a plan needs the hierarchy, because knowing where the task sits
/// is half of what it decides; one that executes an already-written plan needs
/// the plan, and the hierarchy is 18 000 characters re-read on every turn to
/// tell it something the plan already settled.
///
/// Measured on #65: SCOPE was 60 363 characters, of which the roadmap, the
/// milestone and the neighbours were 19 397.
#[derive(Debug, Clone, Copy)]
pub enum Brief<'a> {
    /// No task — the stage opens or plans one rather than working on one.
    ///
    /// An empty ISSUE block would give it something to hunt for.
    Unscoped,
    /// The task, and the roadmap, milestone and neighbours around it.
    Situated(&'a Scope),
    /// The task alone, with its neighbours named by number and nothing else.
    ///
    /// For a stage working against a spec someone else already wrote. The
    /// block says how to fetch a neighbour — `gh issue view` — so the cut is a
    /// cheaper default rather than a removal.
    TaskOnly(&'a Scope),
}

/// Carried by the state of a workflow whose sessions work on an issue.
///
/// It is the bound that makes true, at compile time, the rule "a session never
/// starts without its scope": a session action that composes a prompt requires
/// `S: Scoped`, so it cannot be written without a scope being available. On the
/// Python side, it was a rule written in a comment that nothing enforced.
pub trait Scoped {
    /// The milestone and task of this round.
    fn scope(&self) -> Scope;
}

/// Replace each known `{name}`, **in a single pass**.
///
/// An inserted value is never re-examined: a `{body}` that arrives *in* an
/// issue body remains literal. An unknown `{name}` also remains literal — these
/// templates are prose written for a model, and they carry braces that do not
/// belong to us.
#[must_use]
pub fn splice(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            let name = &after[..end];
            if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
                // Pushed to output, so out of reach of subsequent replacements.
                // That is the whole point.
                out.push_str(value);
            } else {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
            rest = &after[end + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The EXECUTION CONTEXT block, integration branch substituted.
#[must_use]
pub fn preamble(branch: &str, injector: &str) -> String {
    PREAMBLE
        .replace("@INJECTOR@", injector)
        .replace("@BRANCH@", branch)
}

fn roadmap_block(scope: &Scope) -> String {
    scope.roadmap.as_ref().map_or_else(String::new, |roadmap| {
        format!(
            "\nROADMAP #{} — {}\n{}\n",
            roadmap.number,
            roadmap.title,
            non_empty(&roadmap.body)
        )
    })
}

/// How many characters of neighbours' gists a prompt carries, at most.
///
/// A backstop, not the thing that decides: the per-sibling cap in
/// `common::hierarchy` is 300 characters, so a milestone of twelve neighbours
/// comes in under this on its own. What this still catches is the milestone
/// nobody sized for — thirty slices, each with a legitimate boundary.
///
/// **Lowered from 12 000 after measuring it decide.** At the old pair of caps,
/// milestone #17's eight neighbours were 7 811 characters of every prompt, and
/// each gist was sitting at its own cap — the budget was choosing the content
/// instead of guarding against an outlier.
pub const GIST_BUDGET: usize = 4_000;

/// What tells a session a sibling's gist was left out for lack of room.
const TRIMMED: &str = "(the gists below were left out: this block hit its budget. Ask for an issue      by number if you need one.)";

/// Indents a gist under its sibling's line, so the list stays readable.
fn indented(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn siblings_list(scope: &Scope) -> String {
    if scope.siblings.is_empty() {
        return "(none listed)".to_string();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut spent = 0usize;
    let mut trimmed = false;
    for task in &scope.siblings {
        let here = if task.number == scope.task.number {
            " <- CURRENT"
        } else {
            ""
        };
        lines.push(format!(
            "- #{} — {} [{}]{here}",
            task.number, task.title, task.status
        ));
        // The current issue's own body is injected whole, further down: its
        // gist here would be the same text twice.
        if task.number == scope.task.number {
            continue;
        }
        let gist = task.gist.trim();
        if gist.is_empty() {
            continue;
        }
        if spent + gist.len() > GIST_BUDGET {
            trimmed = true;
            continue;
        }
        spent += gist.len();
        lines.push(indented(gist));
    }
    if trimmed {
        lines.push(TRIMMED.to_string());
    }
    lines.join("\n")
}

/// The SCOPE block of a stage: the roadmap, the milestone, its tasks, then
/// the task.
#[must_use]
pub fn scope_block(scope: &Scope, injector: &str) -> String {
    splice(
        SCOPE,
        &[
            ("injector", injector),
            ("roadmap_block", &roadmap_block(scope)),
            ("siblings", &siblings_list(scope)),
            ("milestone", &scope.milestone.number),
            ("milestone_title", &scope.milestone.title),
            ("milestone_body", &non_empty(&scope.milestone.body)),
            ("num", &scope.task.number),
            ("title", &scope.task.title),
            ("body", &non_empty(&scope.task.body)),
        ],
    )
}

/// The task on its own: no roadmap, no milestone body, no neighbours' gists.
///
/// For a stage that executes a spec rather than deciding one — see
/// [`Brief::TaskOnly`]. The neighbours are named by number inside the block, so
/// a session that needs one fetches it in a turn instead of having had it
/// re-read on every turn of the stage.
#[must_use]
pub fn task_block(scope: &Scope, injector: &str) -> String {
    splice(
        TASK,
        &[
            ("injector", injector),
            ("milestone", &scope.milestone.number),
            ("num", &scope.task.number),
            ("title", &scope.task.title),
            ("body", &non_empty(&scope.task.body)),
        ],
    )
}

/// The same hierarchy without the current issue's body, for a prompt that
/// carries that body itself (the refinement).
#[must_use]
pub fn hierarchy_block(scope: &Scope, injector: &str) -> String {
    splice(
        HIERARCHY,
        &[
            ("injector", injector),
            ("roadmap_block", &roadmap_block(scope)),
            ("siblings", &siblings_list(scope)),
            ("milestone", &scope.milestone.number),
            ("milestone_title", &scope.milestone.title),
            ("milestone_body", &non_empty(&scope.milestone.body)),
            ("num", &scope.task.number),
        ],
    )
}

fn non_empty(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        EMPTY_BODY.to_string()
    } else {
        trimmed.to_string()
    }
}

/// The repository's configuration, as a block, or nothing at all.
///
/// Nothing rather than a stated absence, unlike [`EMPTY_BODY`]: an absent
/// digest asks the session for no gesture and changes nothing about its work —
/// it reads the config itself, which is what it did before this block existed.
/// A paragraph explaining that an unknown stack was not recognised would be
/// paid for on every turn to say "carry on as usual".
#[must_use]
pub fn stack_block(stack: &str, injector: &str) -> String {
    if stack.trim().is_empty() {
        return String::new();
    }
    splice(STACK, &[("injector", injector), ("files", stack)])
}

/// The public signatures block, or nothing at all.
///
/// Nothing when the task names no indexed file — the normal case for a task that
/// creates rather than extends. An absent block asks for no gesture, exactly as
/// in [`stack_block`].
#[must_use]
pub fn signatures_block(signatures: &str, injector: &str) -> String {
    if signatures.trim().is_empty() {
        return String::new();
    }
    splice(SIGNATURES, &[("injector", injector), ("files", signatures)])
}

/// A stage's `extra`: its instructions, the repository's configuration, then the
/// scope it works on.
///
/// The scope comes last because it is the long part — the instructions
/// stay where a reader, and a model, find them: at the top. The configuration
/// sits between them: it is reference material, not the brief, and putting it
/// after the issue would leave the session's own task buried in the middle.
///
/// `brief` says which of the three shapes the scope block takes, including the
/// shape that is no block at all — see [`Brief`]. Which one a stage gets is the
/// caller's decision: this layer knows the shapes, not the stages.
#[must_use]
pub fn extra_for(
    instructions: &str,
    brief: Brief<'_>,
    stack: &str,
    signatures: &str,
    injector: &str,
) -> String {
    let (found, block) = match brief {
        Brief::Unscoped => return instructions.to_string(),
        Brief::Situated(found) => (found, scope_block(found, injector)),
        Brief::TaskOnly(found) => (found, task_block(found, injector)),
    };
    let filled = splice(
        instructions,
        &[
            ("num", &found.task.number),
            ("title", &found.task.title),
            ("milestone", &found.milestone.number),
        ],
    );
    let parts = [
        filled,
        stack_block(stack, injector),
        signatures_block(signatures, injector),
        block,
    ];
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// A stage's complete prompt: the command, the preamble, then the extra.
#[must_use]
pub fn build(lead: &str, branch: &str, extra: &str, injector: &str) -> String {
    let mut prompt = format!("{lead}\n{}", preamble(branch, injector));
    if !extra.is_empty() {
        prompt.push('\n');
        prompt.push_str(extra);
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::markers;

    fn many(count: usize, gist_len: usize) -> Scope {
        let mut here = scope();
        here.siblings = (1..=count)
            .map(|n| Sibling {
                number: (100 + n).to_string(),
                title: format!("task {n}"),
                status: "open".to_string(),
                gist: "x".repeat(gist_len),
            })
            .collect();
        here
    }

    #[test]
    fn a_neighbour_is_listed_with_what_it_covers() {
        let said = scope_block(&scope(), "test");
        assert!(said.contains("- #33 — The sibling [done]"));
        assert!(said.contains("covers the fence, not the gate"));
    }

    #[test]
    fn the_current_issue_gist_is_not_repeated_above_its_own_body() {
        // #34 is the current issue: its body is injected whole further down,
        // and the same text twice is the cheapest kind of waste.
        let said = scope_block(&scope(), "test");
        assert_eq!(said.matches("the SPEC").count(), 1, "{said}");
    }

    #[test]
    fn the_block_stops_at_its_budget_and_says_so() {
        let said = scope_block(&many(20, 2_000), "test");
        let gists = said.matches(&"x".repeat(2_000)).count();
        assert!(gists > 0, "some neighbours keep their gist");
        assert!(gists * 2_000 <= GIST_BUDGET);
        assert!(said.contains("hit its budget"), "{said}");
        // Every title is still there: the budget trims gists, never the list.
        for n in 1..=20 {
            assert!(
                said.contains(&format!("task {n} [open]")),
                "task {n} missing"
            );
        }
    }

    #[test]
    fn a_milestone_that_fits_says_nothing_about_a_budget() {
        let said = scope_block(&many(6, 150), "test");
        assert!(!said.contains("hit its budget"), "{said}");
    }

    fn scope() -> Scope {
        Scope {
            roadmap: Some(Named {
                number: "5".to_string(),
                title: "The big plan".to_string(),
                body: "what the roadmap says".to_string(),
            }),
            siblings: vec![
                Sibling {
                    number: "33".to_string(),
                    title: "The sibling".to_string(),
                    status: "done".to_string(),
                    gist: "covers the fence, not the gate".to_string(),
                },
                Sibling {
                    number: "34".to_string(),
                    title: "The grid".to_string(),
                    status: "open".to_string(),
                    gist: "the SPEC".to_string(),
                },
            ],
            milestone: Named {
                number: "12".to_string(),
                title: "The cat".to_string(),
                body: "what the milestone says".to_string(),
            },
            task: Named {
                number: "34".to_string(),
                title: "The grid".to_string(),
                body: "the SPEC".to_string(),
            },
        }
    }

    // --- the two halves of the verbal contract ----------------------------

    #[test]
    fn the_preamble_names_the_same_markers_the_domain_reads() {
        // Two halves of the same contract: if one changes name without
        // the other, a session would say "done" in a language the harness
        // no longer reads.
        let said = preamble("main_agent", INJECTOR);
        assert!(said.contains(markers::OK), "the preamble must require OK");
        assert!(said.contains(markers::STOP), "and offer STOP");
    }

    #[test]
    fn the_integration_branch_replaces_every_occurrence() {
        let said = preamble("main_agent", INJECTOR);
        assert!(!said.contains("@BRANCH@"));
        // Three mentions in rule 3: the PR base, the starting point,
        // and the command. Missing one would send a PR to `main`.
        assert_eq!(said.matches("main_agent").count(), 3);
    }

    #[test]
    fn the_injector_is_named_rather_than_left_as_a_placeholder() {
        let said = preamble("main_agent", "harness/src/main.rs");
        assert!(said.contains("injected by harness/src/main.rs"));
        assert!(!said.contains("@INJECTOR@"));
    }

    // --- single-pass substitution ----------------------------------------

    #[test]
    fn an_untrusted_value_carrying_a_placeholder_stays_literal() {
        // The failure mode this prevents: an issue body that contains
        // `{body}` was being replaced by the next pass.
        let out = splice(
            "A={a} B={b}",
            &[("a", "this contains {b} literally"), ("b", "REPLACED")],
        );
        assert_eq!(out, "A=this contains {b} literally B=REPLACED");
    }

    #[test]
    fn an_unknown_placeholder_is_left_alone() {
        // Prose written for a model carries its own braces.
        assert_eq!(
            splice("keep {this} and set {a}", &[("a", "that")]),
            "keep {this} and set that"
        );
    }

    #[test]
    fn an_unclosed_brace_does_not_swallow_the_rest() {
        assert_eq!(splice("before { after", &[("a", "x")]), "before { after");
    }

    #[test]
    fn a_template_without_any_brace_comes_back_unchanged() {
        assert_eq!(splice("nothing to do", &[("a", "x")]), "nothing to do");
    }

    // --- the scope block ------------------------------------------------

    #[test]
    fn the_scope_block_carries_both_issues_verbatim() {
        let said = scope_block(&scope(), INJECTOR);
        assert!(said.contains("MILESTONE #12 — The cat"));
        assert!(said.contains("what the milestone says"));
        assert!(said.contains("CURRENT ISSUE #34 — The grid"));
        assert!(said.contains("the SPEC"));
    }

    #[test]
    fn the_whole_hierarchy_is_there_and_the_work_is_confined_to_the_current_task() {
        let said = scope_block(&scope(), INJECTOR);
        assert!(said.contains("ROADMAP #5 — The big plan"));
        assert!(said.contains("what the roadmap says"));
        assert!(said.contains("- #33 — The sibling [done]"));
        assert!(said.contains("- #34 — The grid [open] <- CURRENT"));
        assert!(said.contains("Only ISSUE #34 is yours to work on"));
    }

    #[test]
    fn the_hierarchy_block_has_no_issue_body_but_marks_the_current_task() {
        let said = hierarchy_block(&scope(), INJECTOR);
        assert!(said.contains("ROADMAP #5"));
        assert!(said.contains("- #34 — The grid [open] <- CURRENT"));
        assert!(!said.contains("the SPEC"));
        assert!(said.contains("Your work is issue #34 alone"));
    }

    #[test]
    fn the_task_only_block_drops_the_hierarchy_and_names_how_to_fetch_it() {
        // 19 397 of #65's 60 363 SCOPE characters were the roadmap, the
        // milestone and the neighbours, re-read on every turn of a stage that
        // executes a plan those three had already settled.
        let said = task_block(&scope(), INJECTOR);
        assert!(said.contains("CURRENT ISSUE #34 — The grid"));
        assert!(said.contains("the SPEC"));
        assert!(!said.contains("ROADMAP"), "{said}");
        assert!(!said.contains("what the milestone says"), "{said}");
        assert!(!said.contains("covers the fence"), "{said}");
        // A cheaper default, not a removal: the way back is in the block.
        assert!(said.contains("gh issue view"), "{said}");
        assert!(said.contains("milestone #12"), "{said}");
    }

    #[test]
    fn the_task_only_block_is_the_shorter_of_the_two() {
        let whole = scope_block(&scope(), INJECTOR);
        let alone = task_block(&scope(), INJECTOR);
        assert!(
            alone.len() < whole.len(),
            "{} vs {}",
            alone.len(),
            whole.len()
        );
    }

    #[test]
    fn a_task_only_brief_still_fills_the_stages_own_placeholders() {
        let said = extra_for(
            "work on #{num} of milestone #{milestone}",
            Brief::TaskOnly(&scope()),
            "",
            "",
            INJECTOR,
        );
        assert!(said.contains("work on #34 of milestone #12"), "{said}");
        assert!(!said.contains("TASKS OF THIS MILESTONE"), "{said}");
    }

    #[test]
    fn without_a_roadmap_the_block_has_no_roadmap_heading() {
        let mut bare = scope();
        bare.roadmap = None;
        assert!(!scope_block(&bare, INJECTOR).contains("ROADMAP #"));
    }

    #[test]
    fn an_empty_body_is_said_in_words_not_left_blank() {
        let mut bare = scope();
        bare.task.body = "   ".to_string();
        let said = scope_block(&bare, INJECTOR);
        assert!(said.contains(EMPTY_BODY));
    }

    #[test]
    fn a_body_that_mentions_another_field_is_not_substituted() {
        // The real case: someone writes `{body}` in the milestone body.
        let mut tricky = scope();
        tricky.milestone.body = "see {body} and {num}".to_string();
        let said = scope_block(&tricky, INJECTOR);
        assert!(said.contains("see {body} and {num}"));
    }

    // --- the extra ---------------------------------------------------------

    #[test]
    fn instructions_come_before_the_scope_because_scope_is_the_long_part() {
        let said = extra_for("do this", Brief::Situated(&scope()), "", "", INJECTOR);
        let instructions = said.find("do this").expect("the instructions");
        let block = said.find("--- SCOPE").expect("the scope");
        assert!(instructions < block);
    }

    #[test]
    fn a_stage_without_instructions_still_gets_its_scope() {
        // /create-test has no instructions of its own, and would otherwise be the only
        // paid session of the round that ignores which task it is testing.
        let said = extra_for("", Brief::Situated(&scope()), "", "", INJECTOR);
        assert!(said.contains("ISSUE #34"));
        assert!(!said.starts_with('\n'));
    }

    #[test]
    fn the_configuration_sits_between_the_instructions_and_the_issue() {
        // Reference material, not the brief: after the instructions, before the
        // task, so the session's own work is not buried in the middle.
        let said = extra_for(
            "do this",
            Brief::Situated(&scope()),
            "<file path=\"a\">x</file>",
            "",
            INJECTOR,
        );
        let instructions = said.find("do this").expect("the instructions");
        let config = said
            .find("--- REPOSITORY CONFIGURATION")
            .expect("the config");
        let block = said.find("--- SCOPE").expect("the scope");
        assert!(instructions < config && config < block, "{said}");
    }

    #[test]
    fn the_signatures_sit_between_the_configuration_and_the_issue() {
        let said = extra_for(
            "do this",
            Brief::Situated(&scope()),
            "<file path=\"a\">x</file>",
            "<signatures path=\"src/a.rs\">pub fn alpha()</signatures>",
            INJECTOR,
        );
        let config = said.find("--- REPOSITORY CONFIGURATION").expect("config");
        let signatures = said.find("--- PUBLIC SIGNATURES").expect("signatures");
        let scope_at = said.find("--- SCOPE").expect("scope");
        assert!(config < signatures && signatures < scope_at, "{said}");
        assert!(said.contains("pub fn alpha()"));
    }

    #[test]
    fn a_task_naming_no_indexed_file_gets_no_signatures_block() {
        let said = extra_for("do this", Brief::Situated(&scope()), "", "   ", INJECTOR);
        assert!(!said.contains("PUBLIC SIGNATURES"), "{said}");
        assert_eq!(signatures_block("", INJECTOR), "");
    }

    #[test]
    fn a_signature_that_mentions_a_field_is_not_substituted() {
        // Signatures come from the repository, like an issue body: a function
        // literally named `files` must survive the splice.
        let said = signatures_block("pub fn files(injector: u8)", INJECTOR);
        assert!(said.contains("pub fn files(injector: u8)"), "{said}");
    }

    #[test]
    fn an_unknown_stack_adds_no_block_at_all() {
        // Not a stated absence: it asks the session for no gesture and changes
        // nothing about its work, so a paragraph saying so would be paid for on
        // every turn to say "carry on as usual".
        let said = extra_for("do this", Brief::Situated(&scope()), "", "", INJECTOR);
        assert!(!said.contains("REPOSITORY CONFIGURATION"), "{said}");
        assert_eq!(stack_block("   \n ", INJECTOR), "");
    }

    #[test]
    fn the_configuration_block_tells_the_session_the_disk_wins() {
        // The one guard against a digest that went stale mid-run: injected
        // context otherwise reads as more authoritative than the repository.
        let said = stack_block("<file path=\"package.json\">{}</file>", INJECTOR);
        assert!(said.contains("the file on disk wins"), "{said}");
        assert!(said.contains("<file path=\"package.json\">{}</file>"));
    }

    #[test]
    fn a_config_file_that_mentions_a_field_is_not_substituted() {
        // Config is arbitrary text from the repository, like an issue body: a
        // script named `{files}` must survive the splice unchanged.
        let said = stack_block("<file path=\"a\">echo {files} {injector}</file>", INJECTOR);
        assert!(said.contains("echo {files} {injector}"), "{said}");
    }

    #[test]
    fn an_unscoped_stage_gets_no_issue_block_to_hunt_for() {
        // A stage that works on no task: giving it an empty ISSUE block
        // would give it something to hunt for.
        let said = extra_for("open the next item", Brief::Unscoped, "", "", INJECTOR);
        assert_eq!(said, "open the next item");
        assert!(!said.contains("ISSUE #"));
    }

    #[test]
    fn instruction_placeholders_are_filled_from_the_task() {
        let said = extra_for(
            "work on #{num} ({title})",
            Brief::Situated(&scope()),
            "",
            "",
            INJECTOR,
        );
        assert!(said.contains("work on #34 (The grid)"));
    }

    // --- composition -------------------------------------------------------

    #[test]
    fn the_prompt_opens_on_the_command_then_the_preamble() {
        let built = build("/code", "main_agent", "", INJECTOR);
        assert!(built.starts_with("/code\n"));
        assert!(built.contains("--- EXECUTION CONTEXT"));
    }

    #[test]
    fn the_extra_comes_after_the_preamble() {
        let built = build("/code", "main_agent", "the instructions", INJECTOR);
        let context = built.find("END EXECUTION CONTEXT").expect("the preamble");
        let extra = built.find("the instructions").expect("the extra");
        assert!(context < extra);
    }
}
