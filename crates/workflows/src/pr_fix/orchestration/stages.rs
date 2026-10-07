//! The repair: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries *is*
//! the execution order: read what broke and consume the request, then ask
//! for the repair.
//!
//! The prompt text lives here, with the table that sends it: an action
//! receives it as a field and never goes to fetch it.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::pr_fix::action::actions::{AskForFix, ReadBreakage};
use crate::pr_fix::action::publish::ConsumeRequest;
use crate::pr_fix::checks::gates::SomethingIsRed;
use crate::pr_fix::config::Config;
use crate::pr_fix::data::state::FixState;
use crate::pr_fix::ports::Ports;

/// The free stage name: what broke, and the request taken.
pub const CONTEXT: &str = "context";
/// The paid repair stage name.
pub const FIX: &str = "fix";

const FIX_PROMPT: &str = r#"Pull request #{num} — "{title}" ({head} -> {base}) has red CI.
This checkout is already on `{head}`, the PR's own branch.

The checks that have failed:
{failing}

What has been said on the PR so far — a reviewer may already have named the
cause, and a human may have asked for something specific:
<pr-comments>
{comments}
</pr-comments>

Diagnose from the real output, never from the check's name alone:
  gh pr checks {num}
  gh run view <the failing run's id> --log-failed
  gh pr diff {num}

Then fix the cause in this working tree, run the project's own gates
locally until they pass, and push to `{head}`:
  git add <the files you changed, by name>
  git commit -m "fix(<scope>): <what the failure was>"
  git push

Hard rules, and they are the point of this run:
- **Fix the cause, not the signal.** Never delete, skip, `#[ignore]`, or
  weaken the failing test or check. Never edit the CI workflow to make it
  pass. A check that is wrong is a finding to report, not a file to edit.
- **Stay inside this PR's scope.** You are making this change's CI green,
  not improving the code around it. An adjacent problem is something you
  name in your answer, not something you commit here.
- **Never merge, never force-push, never rebase onto another branch, never
  close the PR, and never touch a `harness:*` label.** Something else
  decides what happens to this PR once it is green.
- **No secret in your output.** Name a credential, never its value, and do
  not echo an environment file.

If the failure cannot be fixed inside this PR's scope — it needs a human
decision, a credential you do not have, or a change in another PR — push
nothing and say so plainly, naming what you found and what it would take.
An honest "this needs a human, here is why" is a successful run; a commit
that hides the problem is not.

End your answer with what you changed, the gates you ran with their real
output, and whether you pushed."#;

/// The free stage: read what broke, then take the request. Costs nothing, so
/// no gate guards it.
#[must_use]
pub fn context(ports: &Ports) -> Stage<FixState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            // Order is the contract: the read decides whether the request is
            // consumed at all, so it comes first.
            actions: vec![
                Box::new(ReadBreakage {
                    gh: Rc::clone(&ports.gh),
                }),
                Box::new(ConsumeRequest {
                    gh: Rc::clone(&ports.gh),
                }),
            ],
        },
    }
}

/// The paid stage: one repair attempt, guarded by "something is actually
/// red".
#[must_use]
pub fn fix(ports: &Ports, config: &Config) -> Stage<FixState> {
    Stage {
        name: FIX.to_string(),
        pre: Some(Gate {
            name: "fix requires",
            checks: vec![Box::new(SomethingIsRed)],
        }),
        post: None,
        body: StageBody::Session {
            spec: config.fix.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskForFix {
                stage: FIX.to_string(),
                template: FIX_PROMPT,
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// Context then fix, in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<FixState>> {
    vec![context(ports), fix(ports, config)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_fix::config::fake as config_fake;
    use crate::pr_fix::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_then_fix() {
        let names: Vec<String> = table(&ports_fake::ports(), &config_fake::config())
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(names, [CONTEXT, FIX]);
    }

    #[test]
    fn the_prompt_carries_every_placeholder_the_action_fills() {
        for placeholder in [
            "{num}",
            "{title}",
            "{head}",
            "{base}",
            "{failing}",
            "{comments}",
        ] {
            assert!(
                FIX_PROMPT.contains(placeholder),
                "{placeholder} missing from FIX_PROMPT"
            );
        }
    }

    #[test]
    fn the_prompt_forbids_the_three_ways_a_repair_could_cheat() {
        // A session told only "make CI green" has three cheap outs: delete
        // the test, edit the workflow, or merge past the check. Each is
        // named in the text on purpose.
        for forbidden in [
            "weaken the failing test",
            "edit the CI workflow",
            "Never merge",
        ] {
            assert!(
                FIX_PROMPT.contains(forbidden),
                "{forbidden:?} missing from FIX_PROMPT"
            );
        }
    }
}
