//! The one thing a test-debt attempt writes on GitHub: the label it was
//! asked by.
//!
//! **Removed before the session is paid for, not after it succeeds** — the
//! reasoning of `pr_fix`'s own `ConsumeRequest`: a file whose mutants no
//! session can catch would otherwise buy one session every poll, forever.
//! One label is one attempt. The next rotation that judges the file and
//! still finds mutants poses the label again (`scripts/test-debt.sh`), and a
//! human may pose it by hand.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;

use crate::common::labels;
use crate::test_debt::data::state::DebtState;

/// Removes `harness:test-debt` from the issue: the request has been taken.
pub struct ConsumeRequest {
    /// What removes the label.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<DebtState> for ConsumeRequest {
    async fn run(&self, ctx: &mut Context<DebtState>) -> Outcome<Verdict> {
        // A dry run writes nothing; an issue naming no file asks nothing,
        // and keeping its label leaves it visible to whoever fixes the title.
        if ctx.settings.dry_run || ctx.state.file.is_none() {
            return Ok(Verdict::Continue);
        }
        let number = ctx.state.issue().number;
        self.gh.remove_label(number, labels::TEST_DEBT).await?;
        ctx.traces.say(&format!(
            "#{number}: {} consumed — one attempt follows",
            labels::TEST_DEBT
        ));
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool, file: Option<&str>) -> Context<DebtState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            DebtState {
                issue: Some(Issue {
                    number: 7,
                    ..Issue::default()
                }),
                file: file.map(str::to_string),
                findings: String::new(),
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn a_named_file_consumes_the_label_before_anything_is_paid() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(false, Some("src/lib.rs"));
        ConsumeRequest {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("consumed");
        assert_eq!(
            gh.writes(),
            [Wrote::Unlabelled(7, labels::TEST_DEBT.to_string())]
        );
    }

    #[tokio::test]
    async fn a_dry_run_or_an_issue_naming_nothing_keeps_its_label() {
        for (dry_run, file) in [(true, Some("src/lib.rs")), (false, None)] {
            let gh = Rc::new(FakeGitHub::default());
            let mut context = ctx(dry_run, file);
            ConsumeRequest {
                gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            }
            .run(&mut context)
            .await
            .expect("nothing to do");
            assert!(gh.writes().is_empty(), "dry_run={dry_run} file={file:?}");
        }
    }
}
