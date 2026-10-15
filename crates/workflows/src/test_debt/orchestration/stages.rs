//! The test debt: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries *is*
//! the execution order: read what the issue says and consume the request,
//! then ask for the tests.
//!
//! The prompt text lives here, with the table that sends it: an action
//! receives it as a field and never goes to fetch it.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::prompts;
use crate::test_debt::action::actions::{AskForTests, ReadDebt};
use crate::test_debt::action::publish::ConsumeRequest;
use crate::test_debt::checks::gates::NamesAFile;
use crate::test_debt::config::Config;
use crate::test_debt::data::state::DebtState;
use crate::test_debt::ports::Ports;

/// The free stage name: what the issue says, and the request taken.
pub const CONTEXT: &str = "context";
/// The paid stage name: the tests that catch the mutants.
pub const COVER: &str = "cover";

pub(crate) const COVER_PROMPT: &str = r#"Issue #{num} is test debt: `cargo mutants` rewrote parts of `{file}` and
every test still passed. What the mutation runs found, oldest first:
<findings>
{findings}
</findings>

This checkout is on `{branch}`, the integration branch. Branch off it:
  git checkout -b test/debt-{num}

1. See what is missed today — the findings above may be stale:
     cargo mutants --workspace --file {file}
   If nothing is missed any more, close the issue saying so
   (`gh issue close {num} --comment "<what you ran and what it said>"`),
   push nothing, and stop.
2. For each missed mutant, write the test that catches it, where the
   project's tests of that code live: the module's `#[cfg(test)] mod tests`,
   or the crate's `tests/`. Assert the behaviour the mutant breaks, not the
   line it sits on.
3. Prove it: rerun `cargo mutants --workspace --file {file}` until it reports
   no missed mutant, then run the project's own gates — format, lint, the
   tests — until they pass.
4. Land it:
     git add <the test files you changed, by name>
     git commit -m "test(<scope>): catch the mutants of {file}"
     git push -u origin test/debt-{num}
     gh pr create --base {branch} --title "test: the mutants of {file}" --body "Closes #{num}"
     gh pr checks --watch
     gh pr merge --squash
   then `gh issue close {num}` if the merge did not close it.

Hard rules, and they are the point of this run:
- **Tests only.** Never change the code under test to make a mutant
  disappear. A new test that exposes a real bug stays in your answer, with
  the bug named, and is not committed red.
- **An equivalent mutant is not a test to write**: one with no observable
  difference goes in `.cargo/mutants.toml`'s `exclude_re`, with the reason in
  a comment beside it, and the PR body says so.
- **Never weaken a gate.** Never delete, skip or `#[ignore]` a test, never
  edit the CI workflow, never touch a `harness:*` label.
- **Never merge red.** If the PR's checks fail and tests alone cannot make
  them green, leave the PR open, say why, and stop.
- **No secret in your output.** Name a credential, never its value.

End your answer with the mutants caught, the ones left and why, the gates
you ran with their real output, and the PR."#;

/// The free stage: read what the issue says, then take the request. Costs
/// nothing, so no gate guards it.
#[must_use]
pub fn context(ports: &Ports) -> Stage<DebtState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            // Order is the contract: the read decides whether the request is
            // consumed at all, so it comes first.
            actions: vec![
                Box::new(ReadDebt {
                    gh: Rc::clone(&ports.gh),
                }),
                Box::new(ConsumeRequest {
                    gh: Rc::clone(&ports.gh),
                }),
            ],
        },
    }
}

/// The paid stage: one attempt at the tests, guarded by "the issue names a
/// file".
#[must_use]
pub fn cover(ports: &Ports, config: &Config) -> Stage<DebtState> {
    Stage {
        name: COVER.to_string(),
        pre: Some(Gate {
            name: "cover requires",
            checks: vec![Box::new(NamesAFile)],
        }),
        post: None,
        body: StageBody::Session {
            turn_cap: None,
            spec: config.cover.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskForTests {
                stage: COVER.to_string(),
                template: config.prompts.text(prompts::COVER, COVER_PROMPT),
                branch: config.branch.clone(),
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// Context then cover, in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<DebtState>> {
    vec![context(ports), cover(ports, config)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_debt::config::fake as config_fake;
    use crate::test_debt::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_then_cover() {
        let names: Vec<String> = table(&ports_fake::ports(), &config_fake::config())
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(names, [CONTEXT, COVER]);
    }

    #[test]
    fn the_prompt_carries_every_placeholder_the_action_fills() {
        for placeholder in ["{num}", "{file}", "{branch}", "{findings}"] {
            assert!(
                COVER_PROMPT.contains(placeholder),
                "{placeholder} missing from COVER_PROMPT"
            );
        }
    }

    #[test]
    fn the_prompt_forbids_the_ways_a_session_could_cheat() {
        // A session told only "catch these mutants" has cheap outs: change
        // the code, ignore a test, edit the CI, merge past a red check.
        for forbidden in [
            "Never change the code under test",
            "`#[ignore]`",
            "edit the CI workflow",
            "Never merge red",
        ] {
            assert!(
                COVER_PROMPT.contains(forbidden),
                "{forbidden:?} missing from COVER_PROMPT"
            );
        }
    }
}
