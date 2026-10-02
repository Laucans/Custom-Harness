//! A gate: multiple checks, the first one that fails wins.

use async_trait::async_trait;

use crate::domain::{Outcome, Verdict};
use crate::execution::data::context::Context;
use crate::execution::traits::Verification;

/// Multiple checks surrounding an executable, at any level — around an action
/// to validate individually, around a stage or round to validate the whole.
pub struct Gate<S> {
    /// Named so a reader can list what is verified without reading the bodies,
    /// and so an observability pass has something to log when each passes.
    pub name: &'static str,
    /// In the order they verify. They assume each other: asking `gh` what
    /// labels exist doesn't make sense until we know it's authenticated.
    pub checks: Vec<Box<dyn Verification<S>>>,
}

impl<S> Gate<S> {
    /// A named gate with nothing to verify.
    #[must_use]
    pub fn empty(name: &'static str) -> Self {
        Self {
            name,
            checks: Vec::new(),
        }
    }
}

#[async_trait(?Send)]
impl<S> Verification<S> for Gate<S> {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        for check in &self.checks {
            let verdict = check.verify(ctx).await?;
            if !matches!(verdict, Verdict::Continue) {
                return Ok(verdict);
            }
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Halt;
    use crate::execution::data::context::Settings;
    use crate::traces::Logbook;

    struct AlwaysSkip;

    #[async_trait(?Send)]
    impl Verification<()> for AlwaysSkip {
        async fn verify(&self, _ctx: &Context<()>) -> Outcome<Verdict> {
            Ok(Verdict::Skip("already done".into()))
        }
    }

    struct AlwaysHalts;

    #[async_trait(?Send)]
    impl Verification<()> for AlwaysHalts {
        async fn verify(&self, _ctx: &Context<()>) -> Outcome<Verdict> {
            Err(Halt::Halted("cannot verify".into()))
        }
    }

    fn ctx() -> Context<()> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            (),
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn the_first_check_that_does_not_continue_wins() {
        let gate = Gate {
            name: "test",
            checks: vec![Box::new(AlwaysSkip)],
        };
        let verdict = gate.verify(&ctx()).await.unwrap();
        assert_eq!(verdict, Verdict::Skip("already done".into()));
    }

    #[tokio::test]
    async fn an_empty_gate_continues() {
        let gate: Gate<()> = Gate::empty("test");
        assert_eq!(gate.verify(&ctx()).await.unwrap(), Verdict::Continue);
    }

    #[tokio::test]
    async fn a_failing_check_propagates_as_a_halt() {
        let gate = Gate {
            name: "test",
            checks: vec![Box::new(AlwaysHalts)],
        };
        assert!(gate.verify(&ctx()).await.is_err());
    }
}
