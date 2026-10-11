//! The arbitration: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** `context` reads the board
//! and settles what needs no judgement; `consume` takes the label, so one
//! request is one arbitration; `arbitrate` asks the session about what is
//! left, and only then; `apply` writes the judgement. Stage names are
//! written here, nowhere else.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::arbiter::action::actions::{AskArbiter, Consume, Resolve};
use crate::arbiter::action::publish::Apply;
use crate::arbiter::checks::gates::{NeedsASession, VerdictParses};
use crate::arbiter::config::Config;
use crate::arbiter::data::state::ArbiterState;
use crate::arbiter::ports::Ports;
use crate::prompts;

/// The context stage name: the board read, the mechanical resolution.
pub const CONTEXT: &str = "context";
/// The consume stage name: the label taken.
pub const CONSUME: &str = "consume";
/// The arbitrate stage name: the session.
pub const ARBITRATE: &str = "arbitrate";
/// The apply stage name: the judgement written.
pub const APPLY: &str = "apply";

/// The session's prompt: which task of the milestone produces each open id.
pub(crate) const ARBITRATE_PROMPT: &str = "You arbitrate one question on GitHub issue #{num} (\"{title}\"), a task of
milestone #{milestone} in this repository. Its technical refinement found the
design could not honour the task's `## Architecture` as the split cut it, and
said why:

<decision>
{decision}
</decision>

The task's place in the architecture, as the split wrote it:
<architecture>
{architecture}
</architecture>

The milestone's tasks, each with what it produces and needs, and whether it
has started:
<tasks>
{tasks}
</tasks>

The ids the repository already holds — an aggregate, a DataCapability, a
Capability, a Micro-UI, a screen — which no task has to produce:
<existing>
{existing}
</existing>

What the harness already settled on its own, before asking you:
{settled}

What is left for you, the ids nobody on the board produces:
{open}

For each one, say which task of the milestone is the natural producer: the
task that shapes the aggregate it belongs to, or the command, Capability or
Micro-UI it is a part of — a task that has **not started**, since one that
has already settled what it produces. That task will produce the id too, and
#{num} will wait on it.

Answer `human` instead — and the human reads your reasoning — whenever:
- no task of the milestone is the natural producer: the id belongs to
  another milestone, or to nobody yet;
- the only candidate has started, or is closed;
- the id's meaning is in question — a Concept, an aggregate that does not
  exist, a boundary between systems;
- you are not sure. Score your confidence from 1 to 5: a `resolve` under 4 is
  read as advice by the human, not applied.

Answer with a JSON object and nothing else:
{\"decision\": \"resolve\" or \"human\", \"confidence\": 1 to 5, \"needs\": [{\"id\": \"<the id, as the manifests spell it>\", \"producer\": <task number>}], \"reason\": \"one or two sentences\"}
`needs` lists every open id with the task that will produce it; empty for
`human`. Name tasks by the numbers above, nothing else.

Change no file, post no comment, touch no issue and no label: the workflow
applies your answer. The issues are public — never write a secret, a token, a
key, a password or a URL carrying one.";

/// The four stages, in order: context, consume, arbitrate, apply.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<ArbiterState>> {
    vec![
        Stage {
            name: CONTEXT.to_string(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(Resolve {
                    disk: Rc::clone(&ports.disk),
                    root: config.root.clone(),
                })],
            },
        },
        Stage {
            name: CONSUME.to_string(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(Consume {
                    gh: Rc::clone(&ports.gh),
                })],
            },
        },
        Stage {
            name: ARBITRATE.to_string(),
            pre: Some(Gate {
                name: "arbitrate requires",
                checks: vec![Box::new(NeedsASession)],
            }),
            post: Some(Gate {
                name: "arbitrate must achieve",
                checks: vec![Box::new(VerdictParses {
                    stage: ARBITRATE.to_string(),
                })],
            }),
            body: StageBody::Session {
                spec: config.arbitrate.clone(),
                sessions: Rc::clone(&ports.sessions),
                actions: vec![Box::new(AskArbiter {
                    stage: ARBITRATE.to_string(),
                    template: config.prompts.text(prompts::ARBITRATE, ARBITRATE_PROMPT),
                    spending: Rc::clone(&ports.spending),
                })],
            },
        },
        Stage {
            name: APPLY.to_string(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(Apply {
                    gh: Rc::clone(&ports.gh),
                    arbitrate: ARBITRATE.to_string(),
                })],
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbiter::config::fake as config_fake;
    use crate::arbiter::ports::fake as ports_fake;

    #[test]
    fn the_table_reads_consumes_arbitrates_then_applies() {
        let names: Vec<String> = table(&ports_fake::ports(), &config_fake::config())
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(names, [CONTEXT, CONSUME, ARBITRATE, APPLY]);
    }

    #[test]
    fn the_prompt_carries_every_placeholder_the_action_fills() {
        for key in [
            "{num}",
            "{title}",
            "{milestone}",
            "{decision}",
            "{architecture}",
            "{tasks}",
            "{existing}",
            "{settled}",
            "{open}",
        ] {
            assert!(ARBITRATE_PROMPT.contains(key), "{key} missing");
        }
    }
}
