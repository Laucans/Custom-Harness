//! Split: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries *is*
//! the execution order: context, then the slice request, then publishing.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::split::action::actions::{AskForSlice, ReadExistingTasks};
use crate::split::action::publish::Write;
use crate::split::checks::gates::SliceParses;
use crate::split::config::Config;
use crate::split::data::state::SplitState;
use crate::split::ports::Ports;

/// The free context-reading stage name.
pub const CONTEXT: &str = "context";
/// The paid slice-request stage name.
pub const SLICE: &str = "slice";
/// The publish local stage name.
pub const PUBLISH: &str = "publish";

const SLICE_PROMPT: &str = r#"Milestone #{num} — "{title}":
{body}

Decide the task slices that deliver this milestone — each one executable by
one fresh session and becoming exactly one SPEC, following
`.claude/skills/split/SKILL.md`'s own task-splitting rules. Give each
slice a short brief — what it covers, and what it must not take from the
next slice — and a branch name of the form `<type>/<slug>` (`feat`, `fix`,
`docs`, `refactor`, `test`, `chore`, `AIchore`). Mark a slice
`needs_human: true` only for account creation, an interactive login, a
payment decision, or anything else only a human in a browser can do —
everything else is `needs_human: false`. Order so something testable exists
early, and make the last slice the one that proves the whole milestone runs
end to end.

{existing}

Answer with a JSON array and nothing else, one object per slice, in the
order tasks must be taken:
```json
[{"title": "a short, specific title", "brief": "what this slice covers and does not cover", "branch": "feat/a-slug", "needs_human": false}]
```
An empty array `[]` means this milestone needs no further task."#;

/// The free context-reading stage: task slices already open. Costs nothing,
/// so no `pre`/`post` gate is needed.
#[must_use]
pub fn context(ports: &Ports) -> Stage<SplitState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(ReadExistingTasks {
                gh: Rc::clone(&ports.gh),
            })],
        },
    }
}

/// The paid slice-request stage: asks for the JSON plan, tolerating one
/// malformed reply (`AskForSlice`), then demands it parses (`SliceParses`).
#[must_use]
pub fn slice(ports: &Ports, config: &Config) -> Stage<SplitState> {
    Stage {
        name: SLICE.to_string(),
        pre: None,
        post: Some(Gate {
            name: "slice must parse",
            checks: vec![Box::new(SliceParses {
                stage: SLICE.to_string(),
            })],
        }),
        body: StageBody::Session {
            spec: config.slice.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskForSlice {
                stage: SLICE.to_string(),
                template: SLICE_PROMPT,
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// Publishing: a local stage, which costs nothing.
#[must_use]
pub fn publish(ports: &Ports, config: &Config) -> Stage<SplitState> {
    Stage {
        name: PUBLISH.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Write {
                gh: Rc::clone(&ports.gh),
                slice_stage: SLICE.to_string(),
                base_branch: config.base_branch.clone(),
            })],
        },
    }
}

/// Context, slice, publish — in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<SplitState>> {
    vec![context(ports), slice(ports, config), publish(ports, config)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::config::fake as config_fake;
    use crate::split::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_then_slice_then_publish() {
        let names: Vec<String> = table(&ports_fake::ports(), &config_fake::config())
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(names, [CONTEXT, SLICE, PUBLISH]);
    }

    #[test]
    fn the_prompt_carries_every_placeholder_the_action_fills() {
        for placeholder in ["{num}", "{title}", "{body}", "{existing}"] {
            assert!(
                SLICE_PROMPT.contains(placeholder),
                "{placeholder} missing from SLICE_PROMPT"
            );
        }
    }
}
