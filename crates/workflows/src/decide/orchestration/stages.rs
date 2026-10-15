//! A decision: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** Read the case and take the
//! request, decide in one session, then write and carry out the decision.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::decide::action::actions::{AskDecider, ReadCase};
use crate::decide::action::publish::{Apply, ConsumeRequest};
use crate::decide::checks::gates::{DecisionParses, NeedsASession};
use crate::decide::config::Config;
use crate::decide::data::state::DecideState;
use crate::decide::ports::Ports;
use crate::prompts;

/// The free stage name: the case read, the request taken.
pub const CONTEXT: &str = "context";
/// The paid stage name: the decision.
pub const DECIDE: &str = "decide";
/// The free stage name: the decision written and carried out.
pub const APPLY: &str = "apply";

pub(crate) const DECIDE_PROMPT: &str = r#"Issue #{num} — "{title}" — needs a decision ({kind}). No human is
reachable: you decide, and the human reads your decision afterwards and may
overrule it. Never ask; never answer "it depends".

The issue's body:
<body>
{body}
</body>

Its `## Decision` as it stands:
<decision>
{decision}
</decision>

What has been said on it, the latest last:
<comments>
{comments}
</comments>

Its pull request: {pr}
What has been said on the pull request — its review among it:
<review>
{review}
</review>

Read the repository where it helps (the design dossier is
`mock/proof-of-concept.json`, `mock/data/aggregates/`, `rules.json`;
the architecture `docs/ARCHITECTURE.md`; the rules' tests `tests/rules/`).
Decide with what is already decided: the design dossier, the rules, the
milestone's plan. Prefer what keeps the gates green and the plan intact.

What you may do here: {actions}.
- `answer`: the question is answered; the task goes on from your answer,
  which the next session reads under `## Decision`. For a session that
  stopped on an ambiguity, answer it the way the design already points to.
- `merge`: the pull request is merged as it is — what its review asks is
  not worth another repair. For the rules' task (`rules`), its tests stand
  as they are written.
- `repair`: the pull request is repaired once more; say in `answer` exactly
  what to change.
- `cut`: the task is closed — it is out of scope, or the plan no longer
  needs it; say why.
For a port a person refused (`port`, the refusal in the comments): `answer`
how the task does without it — inside the product, or through a port
ports.json already has — or `cut` it when nothing of it stands without.
Never decide what only a person can do — spending money, creating an
account, a credential: `answer` that it waits on a `harness:human` task, and
name it.

Answer with ONLY a JSON object:
{"action": "answer", "summary": "the decision, one sentence", "answer": "what the next session reads: the choices made, markdown", "confidence": 4}
`confidence` is how sure you are, 1 to 5."#;

/// The free stage: read the case, then take the request.
#[must_use]
pub fn context(ports: &Ports) -> Stage<DecideState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![
                Box::new(ReadCase {
                    gh: Rc::clone(&ports.gh),
                }),
                Box::new(ConsumeRequest {
                    gh: Rc::clone(&ports.gh),
                }),
            ],
        },
    }
}

/// The paid stage: one session decides.
#[must_use]
pub fn decide(ports: &Ports, config: &Config) -> Stage<DecideState> {
    Stage {
        name: DECIDE.to_string(),
        pre: Some(Gate {
            name: "decide requires",
            checks: vec![Box::new(NeedsASession)],
        }),
        post: Some(Gate {
            name: "decide must answer",
            checks: vec![Box::new(DecisionParses {
                stage: DECIDE.to_string(),
            })],
        }),
        body: StageBody::Session {
            spec: config.decide.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskDecider {
                stage: DECIDE.to_string(),
                template: config.prompts.text(prompts::DECIDE, DECIDE_PROMPT),
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// The free stage: write, carry out, label, tell.
#[must_use]
pub fn apply(ports: &Ports) -> Stage<DecideState> {
    Stage {
        name: APPLY.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Apply {
                gh: Rc::clone(&ports.gh),
                decide_stage: DECIDE.to_string(),
            })],
        },
    }
}

/// Context, decide, apply, in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<DecideState>> {
    vec![context(ports), decide(ports, config), apply(ports)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decide::config::fake as config_fake;
    use crate::decide::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_decide_apply() {
        let table = table(&ports_fake::ports(), &config_fake::config());
        let names: Vec<&str> = table.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [CONTEXT, DECIDE, APPLY]);
        assert!(table[1].pre.is_some() && table[1].post.is_some());
    }

    #[test]
    fn the_prompt_decides_never_asks_and_never_spends() {
        for placeholder in [
            "{num}",
            "{title}",
            "{kind}",
            "{body}",
            "{decision}",
            "{comments}",
            "{pr}",
            "{review}",
            "{actions}",
        ] {
            assert!(DECIDE_PROMPT.contains(placeholder), "{placeholder}");
        }
        for said in [
            "Never ask",
            "harness:human",
            "ONLY a JSON object",
            "`cut`",
            "`repair`",
        ] {
            assert!(DECIDE_PROMPT.contains(said), "{said}");
        }
    }
}
