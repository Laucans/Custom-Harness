//! The rules' task: the sequence, and what we know of each stage.
//!
//! **The only design surface of the workflow.** The order of entries *is*
//! the execution order: read what the issue asks and take the request, then
//! write the rules and their tests, then judge them in a session of its own.
//!
//! The prompt texts live here, with the table that sends them.

use std::rc::Rc;

use harness_core::execution::{Gate, Stage, StageBody};

use crate::prompts;
use crate::rules::action::actions::{AskToJudge, AskToWrite, ReadRequest};
use crate::rules::action::publish::ConsumeRequest;
use crate::rules::checks::gates::AsksSomething;
use crate::rules::config::Config;
use crate::rules::data::state::RulesState;
use crate::rules::ports::Ports;

/// The free stage name: what the issue asks, and the request taken.
pub const CONTEXT: &str = "context";
/// The first paid stage name: the rules and their tests written.
pub const WRITE: &str = "write";
/// The second paid stage name: each test judged against its criterion.
pub const FIDELITY: &str = "fidelity";

pub(crate) const WRITE_PROMPT: &str = r#"Issue #{num} is the rules' task. The rules below were validated by the
human while designing the product; each becomes `rules.json` (contract P,
`contracts/p-rules.schema.json`) and tests that prove it — before any of the
product is built. The code is written afterwards to make these tests pass,
and no development task starts while this issue is open.

<rules>
{rules}
</rules>

Changed rules, marked `conform` (their tests run ahead of the code): {conform}

What the issue's comments say — a former attempt's review, the human's
answer — oldest first:
<comments>
{comments}
</comments>

This checkout is on `{branch}`, the integration branch. If a pull request
from `rules/{num}` is open (`gh pr list --head rules/{num}`), check that
branch out and answer its review; otherwise branch off:
  git checkout -b rules/{num}

1. Write `rules.json` at the repository's root exactly as given above.
2. Read `tests/rules/business/src/lib.rs` and `tests/rules/journey/kit.ts`:
   they say how a rule's test is written. For each rule, its tests, in its
   family's suite:
   - business: `tests/rules/business/tests/<id>_<slug>.rs`, a `#[test]`
     calling `rules_business::rule("<id>", &[<needs>], |s| ...)`;
   - journey: `tests/rules/journey/<id>-<slug>.spec.ts`, calling
     `rule('<id>', [<needs>], '<title>', async ({ page }) => ...)`.
   Above each test, its rule's criterion as three comment lines, copied as
   they are: `// given: ...`, `// when: ...`, `// then: ...`.
   A test reaches the product only by identifier, the way a Micro-UI does:
   the Capabilities, DataCapabilities, Micro-UIs and screens the
   architecture (`ARCHITECTURE.md`) and the mock-up (`mock/pages.json`)
   name. It lists every unit it uses in its needs (`capability:<Id>`,
   `data-capability:<Name>`, `micro-ui:<name>`, `screen:<name>`,
   `aggregate:<Name>`), so it stays pending until they are built. Its given
   comes from the mock-up's fixtures (`fixture("<name>")`).
   A rule no longer in the list has its tests removed; a changed rule's
   tests are rewritten to its new criterion.
3. Prove it: `node scripts/rules-gate.mjs coverage` passes; the business
   suite builds and runs (`cargo test --manifest-path
   tests/rules/business/Cargo.toml`, a pending test passes); the journey
   suite lists its tests (`cd tests/rules/journey && npm install && npx
   playwright test --list`).
4. Open the change, and do not merge it — another reader judges it:
     git add rules.json tests/rules
     git commit -m "test(rules): the rules and their tests (#{num})"
     git push -u origin rules/{num}
     gh pr create --base {branch} --label harness:rules --title "The rules and their tests (#{num})" --body "Closes #{num}"

Hard rules, and they are the point of this run:
- **rules.json and tests/rules/ only.** Never write the product's code,
  never touch the CI, the gates, `scripts/`, or a `harness:*` label.
  The mock server's handlers are their units' own files
  (`mock/handlers/<id>.js`): a given starts from the fixtures, never from
  a handler you write or edit there, nor in mock/src/reads.js or
  mock/src/commands.js.
- **The criterion, nothing weaker and nothing more.** A test does its given,
  then its when, and asserts its then — never an assertion that cannot
  fail, never one the criterion does not make.
- **Never merge.** The second reader does.
- **No secret in your output.** Name a credential, never its value.

End your answer with each rule and its tests, the checks you ran with their
real output, and the pull request."#;

pub(crate) const CONTEST_PROMPT: &str = r#"Issue #{num} contests a rule's test: a development found it cannot meet
it and wrote `CONTEST.md` instead of changing it. What it said, as the CI
copied it:
<contest>
{said}
</contest>

What the issue's comments say, oldest first:
<comments>
{comments}
</comments>

You decide; the human is warned and may act afterwards. Read `rules.json`,
the rule's criterion, its tests under `tests/rules/`, and the development's
pull request (`gh pr view <n>`, `gh pr diff <n>`).

One answer only:
a. **The test misreads its rule** — it asserts more, less or other than its
   criterion: fix the test.
b. **The rule cannot hold** — it contradicts another rule or the
   architecture: change the rule in `rules.json` the smallest way that lets
   it hold, and its tests to match. The pull request's body says plainly
   what changed in the rule and why: the human reads it.
c. **The test stands**: change nothing.

For a and b, on a branch off `{branch}`:
  git checkout -b rules/{num}
run `node scripts/rules-gate.mjs coverage` until it passes, then commit,
push, and open the change without merging it — another reader judges it:
  gh pr create --base {branch} --label harness:rules --title "Rules contest settled (#{num})" --body "Closes #{num} — <the decision, and why>"
For c, close the issue with the decision and why:
  gh issue close {num} --comment "<the decision, and why>"

In every case, comment on the development's pull request: the decision,
and that it removes `CONTEST.md` and conforms to the test as it lands.

Hard rules:
- **rules.json and tests/rules/ only.** Never write the product's code,
  never touch the CI, the gates, `scripts/`, or a `harness:*` label.
- **Never weaken a test to let the development through**: a test is
  changed only when it misreads its rule, a rule only when it cannot hold.
- **Never merge.** The second reader does.
- **No secret in your output.**

End your answer with the decision, why, and what changed."#;

pub(crate) const FIDELITY_PROMPT: &str = r"You judge the rules' tests another session wrote for issue #{num}. You did
not write them, and you do not change them. Their pull request comes from
`rules/{num}` (`gh pr list --head rules/{num} --json number,url`). If there
is none and the issue is closed, the task needed no change: stop, saying so.

The rules as the issue gives them:
<rules>
{rules}
</rules>

For each rule of the pull request's `rules.json`, read its tests
(`gh pr diff <n>`, or check the branch out):
1. `node scripts/rules-gate.mjs coverage` passes on the branch: every rule
   has a test of its family, its criterion copied above it as it is.
2. The test does the given, then the when, then asserts the then — nothing
   weaker (an assertion that cannot fail, a then checked before the when, a
   when never performed), nothing more (an assertion the criterion does
   not make).
3. It reaches the product only by identifier, and lists in its needs every
   unit it uses.
4. The pull request touches `rules.json` and `tests/rules/` only.

Every test faithful: comment on the pull request one line saying so, and
end that comment with the line `<!-- rules-fidelity: faithful -->`. Do not
merge and do not wait for its checks: the harness merges it once they are
green, and closes the issue. A rule marked `conform` may be red: its code
comes after.
Anything unfaithful: do not merge. Comment on the pull request each
finding — the rule, the test, what is wrong, what would be faithful — and
end that comment with the line `<!-- rules-fidelity: unfaithful -->`. Then
read the issue's comments (`gh issue view {num} --comments`):
- if no earlier review of yours found it unfaithful (no such line on the
  pull request before yours), comment on the issue one line pointing to
  your findings and `gh issue edit {num} --add-label harness:ready`: the
  writer answers them;
- if one already did, the writer did not settle it: comment on the issue
  one line pointing to both, then
  `gh issue edit {num} --add-label harness:needs-decision` — the harness
  decides between them.
Then stop.

No secret in your output. End your answer with each rule and your verdict
on each of its tests.";

/// The free stage: read what the issue asks, then take the request. Costs
/// nothing, so no gate guards it.
#[must_use]
pub fn context(ports: &Ports) -> Stage<RulesState> {
    Stage {
        name: CONTEXT.to_string(),
        pre: None,
        post: None,
        body: StageBody::Local {
            // Order is the contract: what cannot be read is said when the
            // request is taken.
            actions: vec![
                Box::new(ReadRequest {
                    gh: Rc::clone(&ports.gh),
                }),
                Box::new(ConsumeRequest {
                    gh: Rc::clone(&ports.gh),
                }),
            ],
        },
    }
}

/// The first paid stage: the rules and their tests, or the contest settled.
#[must_use]
pub fn write(ports: &Ports, config: &Config) -> Stage<RulesState> {
    Stage {
        name: WRITE.to_string(),
        pre: Some(Gate {
            name: "write requires",
            checks: vec![Box::new(AsksSomething)],
        }),
        post: None,
        body: StageBody::Session {
            turn_cap: None,
            spec: config.write.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskToWrite {
                stage: WRITE.to_string(),
                write: config.prompts.text(prompts::RULES_WRITE, WRITE_PROMPT),
                contest: config.prompts.text(prompts::RULES_CONTEST, CONTEST_PROMPT),
                branch: config.branch.clone(),
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// The second paid stage: a fresh session judges each test and merges.
#[must_use]
pub fn fidelity(ports: &Ports, config: &Config) -> Stage<RulesState> {
    Stage {
        name: FIDELITY.to_string(),
        pre: Some(Gate {
            name: "fidelity requires",
            checks: vec![Box::new(AsksSomething)],
        }),
        post: None,
        body: StageBody::Session {
            turn_cap: None,
            spec: config.fidelity.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskToJudge {
                stage: FIDELITY.to_string(),
                template: config
                    .prompts
                    .text(prompts::RULES_FIDELITY, FIDELITY_PROMPT),
                branch: config.branch.clone(),
                spending: Rc::clone(&ports.spending),
            })],
        },
    }
}

/// Context, write, fidelity, in order.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<RulesState>> {
    vec![
        context(ports),
        write(ports, config),
        fidelity(ports, config),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::config::fake as config_fake;
    use crate::rules::ports::fake as ports_fake;

    #[test]
    fn the_order_of_the_table_is_context_write_fidelity() {
        let table = table(&ports_fake::ports(), &config_fake::config());
        let names: Vec<&str> = table.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [CONTEXT, WRITE, FIDELITY]);
        assert!(table[0].pre.is_none(), "the free stage is never gated");
        assert!(table[1].pre.is_some() && table[2].pre.is_some());
    }

    #[test]
    fn each_prompt_carries_the_placeholders_the_actions_fill() {
        for (prompt, needs) in [
            (
                WRITE_PROMPT,
                &["{num}", "{branch}", "{rules}", "{conform}", "{comments}"][..],
            ),
            (
                CONTEST_PROMPT,
                &["{num}", "{branch}", "{said}", "{comments}"][..],
            ),
            (FIDELITY_PROMPT, &["{num}", "{rules}"][..]),
        ] {
            for need in needs {
                assert!(prompt.contains(need), "{need}");
            }
        }
    }

    #[test]
    fn the_rules_task_leaves_every_unit_s_mock_handlers_to_its_unit() {
        let flat = WRITE_PROMPT
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(flat.contains(
            "The mock server's handlers are their units' own files (`mock/handlers/<id>.js`)"
        ));
        assert!(flat.contains("nor in mock/src/reads.js or mock/src/commands.js"));
    }

    #[test]
    fn the_writers_never_merge_and_only_the_judge_does() {
        for prompt in [WRITE_PROMPT, CONTEST_PROMPT] {
            assert!(prompt.contains("**Never merge.**"));
            assert!(!prompt.contains("gh pr merge"));
            assert!(prompt.contains("rules.json and tests/rules/ only"));
            assert!(prompt.contains("--label harness:rules"));
        }
        assert!(!FIDELITY_PROMPT.contains("gh pr merge"));
        assert!(FIDELITY_PROMPT.contains("<!-- rules-fidelity: faithful -->"));
        assert!(FIDELITY_PROMPT.contains("do not merge"));
        assert!(FIDELITY_PROMPT.contains("--add-label harness:ready"));
        assert!(FIDELITY_PROMPT.contains("--add-label harness:needs-decision"));
        assert!(
            !FIDELITY_PROMPT.contains("harness:human"),
            "never waits on a human"
        );
        assert!(WRITE_PROMPT.contains("node scripts/rules-gate.mjs coverage"));
        assert!(CONTEST_PROMPT.contains("Never weaken a test"));
    }
}
