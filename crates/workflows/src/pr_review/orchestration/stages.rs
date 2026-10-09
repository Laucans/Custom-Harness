//! The review: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** Two paid passes, then
//! publishing — which is a table stage but not a session, because the
//! sequence chains the two kinds.
//!
//! The pass text lives here, with the table that sends it: an action
//! receives it as a field and never goes to fetch it.
//!
//! Models are divided by work type. `/code` already ran an adverse review
//! before merging: pass 1 is a second opinion in fresh context, not the
//! only line of defense. Pass 2 summarizes what pass 1 and the diff already
//! say — it's writing, not a hunt.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::pr_review::action::actions::{AskBrief, AskInline};
use crate::pr_review::action::publish::Publish;
use crate::pr_review::checks::gates::{InlinePassIsOff, NothingIsPosted};
use crate::pr_review::config::Config;
use crate::pr_review::data::state::ReviewState;
use crate::pr_review::ports::Ports;

/// The name of the line-by-line pass stage.
///
/// **Stage names are written here, nowhere else.** Gates and actions
/// receive them as fields: two literals in two layers would silently
/// desynchronize — the summary would receive "the pass left nothing" when
/// it ran.
pub const INLINE: &str = "inline";
/// The name of the brief stage.
pub const BRIEF: &str = "brief";
/// The name of the local publishing stage.
pub const PUBLISH: &str = "publish";

const BRIEF_PROMPT: &str =
    "You are writing review notes on pull request #{num} (\"{title}\", {head} -> {base})
in this repository. The branch was written by an unattended agent loop: a
human is about to read the diff for the first time and has to decide whether
to trust it. Your notes are the only orientation they get.

Start by reading the change and its intent:
  gh pr diff {num}
  gh pr view {num} --json title,body,commits

You also need the spec the branch was built from. It is the body of the
GitHub issue this PR closes — the PR body carries `Closes #<n>` on its own
line:
  gh issue view <n> --json title,body
Read the milestone issue it hangs under only if you need to place the task.

The spec is not committed alongside the code, so it cannot drift inside the
diff. What is worth checking instead is the gap between what the issue asks
for and what the branch does: scope quietly dropped, or quietly widened, is
the drift this comment exists to surface.

A line-by-line bug hunt already ran and posted its findings inline. Here is
what it reported — reference it, do not repeat it:
<inline-review-output>
{findings}
</inline-review-output>

Write ONE markdown comment body, and output nothing else — no preamble, no
\"here is the comment\", no code fence around the whole thing. Around 200-350
words, these sections, dropping any that would be empty:

**What this batch does** — the change in 2-4 sentences, in intent terms, not a
file listing.

**Worth checking first** — the 2-4 places where a human's attention is
actually worth spending, each as `file.ts:line` (markdown link relative to
the repo root) plus one line on why. Rank them; do not list everything.

**Gaps with the SPEC** — anything the spec asked for that is not here, or
here but not asked for. Say \"conformant\" if it matches.

**Assumptions made** — decisions the agent made that the spec left open,
and that a human might have made differently. This is the section that most
often matters: the loop guesses silently.

**Questions** — what you would ask the author. Omit if you have none.

Write clearly, the way a colleague leaves review notes. Be concrete and
specific to this diff — no generic advice, no praise, no summary of your own
process. If the change is small and clean, say so briefly rather than
inflating it. Do not edit any file and do not post anything yourself; the
script posts what you output.

{strictness}
End the comment with ONE last line, alone, that the loop reads to decide
what happens next:
  VERDICT: blocking — <the defect, in a few words>
when the batch must not be merged as it is: a correctness bug, a security
hole, a gate or test that can pass without checking what it claims, or a
spec requirement missing. A repair session will be sent with your notes.
  VERDICT: clean
otherwise — style, naming, simplifications, nice-to-haves and open questions
never block. When in doubt, it is clean: a blocking verdict costs a repair
round.";

/// The line-by-line pass: `/code-review`, skipped under `--no-inline`.
#[must_use]
pub fn inline(ports: &Ports, config: &Config) -> Stage<ReviewState> {
    Stage {
        name: INLINE.to_string(),
        pre: Some(Gate {
            name: "inline requires",
            checks: vec![Box::new(InlinePassIsOff {
                no_inline: config.no_inline,
            })],
        }),
        post: None,
        body: StageBody::Session {
            spec: config.inline.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskInline {
                stage: INLINE.to_string(),
                level: config.level.clone(),
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// The summary pass: reads `inline`, writes notes for the human.
#[must_use]
pub fn brief(ports: &Ports, config: &Config) -> Stage<ReviewState> {
    Stage {
        name: BRIEF.to_string(),
        pre: None,
        post: None,
        body: StageBody::Session {
            spec: config.brief.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskBrief {
                stage: BRIEF.to_string(),
                inline_stage: INLINE.to_string(),
                template: BRIEF_PROMPT,
                no_inline: config.no_inline,
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// Publishing: a local stage, which costs nothing.
#[must_use]
pub fn publish(ports: &Ports, config: &Config) -> Stage<ReviewState> {
    Stage {
        name: PUBLISH.to_string(),
        pre: Some(Gate {
            name: "publish requires",
            checks: vec![Box::new(NothingIsPosted)],
        }),
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Publish {
                gh: Rc::clone(&ports.gh),
                costs: Rc::clone(&ports.costs),
                disk: Rc::clone(&ports.disk),
                brief: BRIEF.to_string(),
                review_dir: config.review_dir.clone(),
                no_inline: config.no_inline,
                level: config.level.clone(),
                inline_model: config.inline.model.clone(),
                brief_model: config.brief.model.clone(),
                now: ports.now,
            })],
        },
    }
}

/// The two passes and publishing, in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<ReviewState>> {
    vec![
        inline(ports, config),
        brief(ports, config),
        publish(ports, config),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_review::config::fake as config_fake;
    use crate::pr_review::ports::fake as ports_fake;

    #[test]
    fn the_table_is_inline_then_brief_then_publish() {
        let names: Vec<String> = table(&ports_fake::ports(), &config_fake::config())
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(names, ["inline", "brief", "publish"]);
    }
}
