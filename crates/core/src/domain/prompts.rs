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
   Repository etiquette stands unchanged — branch -> PR ->
   `gh pr merge --rebase`, and never a direct push.
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
The milestone and the issue below are the whole brief; there is no
docs/current/ any more. Both are GitHub issues: what you produce goes back
into the issue, not into a file under docs/.

MILESTONE #{milestone} — {milestone_title}
{milestone_body}

ISSUE #{num} — {title}
{body}
--- END SCOPE ---";

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

/// What a session works on: a milestone, and a task in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// The milestone.
    pub milestone: Named,
    /// The task.
    pub task: Named,
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

/// The SCOPE block of a stage: the milestone, then the task.
#[must_use]
pub fn scope_block(scope: &Scope, injector: &str) -> String {
    let milestone_body = non_empty(&scope.milestone.body);
    let body = non_empty(&scope.task.body);
    splice(
        SCOPE,
        &[
            ("injector", injector),
            ("milestone", &scope.milestone.number),
            ("milestone_title", &scope.milestone.title),
            ("milestone_body", &milestone_body),
            ("num", &scope.task.number),
            ("title", &scope.task.title),
            ("body", &body),
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

/// A stage's `extra`: its instructions, then the scope it works on.
///
/// The scope comes last because it is the long part — the instructions
/// stay where a reader, and a model, find them: at the top.
///
/// `scope` is an `Option` for **one** case only, and there is a reason: the
/// rollover stage (`/planner`) works on no task, it opens one. Giving it
/// an empty ISSUE block would give it something to hunt for. All others
/// get one, including those with no instructions of their own.
#[must_use]
pub fn extra_for(instructions: &str, scope: Option<&Scope>, injector: &str) -> String {
    let Some(found) = scope else {
        return instructions.to_string();
    };
    let filled = splice(
        instructions,
        &[
            ("num", &found.task.number),
            ("title", &found.task.title),
            ("milestone", &found.milestone.number),
        ],
    );
    let block = scope_block(found, injector);
    if filled.is_empty() {
        block
    } else {
        format!("{filled}\n\n{block}")
    }
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

    fn scope() -> Scope {
        Scope {
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
        assert!(said.contains("ISSUE #34 — The grid"));
        assert!(said.contains("the SPEC"));
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
        let said = extra_for("do this", Some(&scope()), INJECTOR);
        let instructions = said.find("do this").expect("the instructions");
        let block = said.find("--- SCOPE").expect("the scope");
        assert!(instructions < block);
    }

    #[test]
    fn a_stage_without_instructions_still_gets_its_scope() {
        // /create-test has no instructions of its own, and would otherwise be the only
        // paid session of the round that ignores which task it is testing.
        let said = extra_for("", Some(&scope()), INJECTOR);
        assert!(said.contains("ISSUE #34"));
        assert!(!said.starts_with('\n'));
    }

    #[test]
    fn the_rollover_stage_gets_no_issue_block_to_hunt_for() {
        // /planner has no task: giving it an empty ISSUE block
        // would give it something to hunt for.
        let said = extra_for("open the next item", None, INJECTOR);
        assert_eq!(said, "open the next item");
        assert!(!said.contains("ISSUE #"));
    }

    #[test]
    fn instruction_placeholders_are_filled_from_the_task() {
        let said = extra_for("work on #{num} ({title})", Some(&scope()), INJECTOR);
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
