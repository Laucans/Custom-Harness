//! The relay between two tasks of one system: a `code` session that ended
//! with a small context is continued by the next task's, which branches
//! from it instead of reading everything again.
//!
//! The owner's rule: a task that read little hands its session on. Each
//! task keeps its own issue, branch, pull request and review — only the
//! conversation is shared, and only forward: the base stays as it was.

use std::rc::Rc;

use async_trait::async_trait;
use harness_architecture::vocabulary::Declaration;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::agent::Base;
use harness_core::ports::store::handoffs::{Handoff, Handoffs};

use crate::common::sections;
use crate::dev_loop::data::state::Loop;

/// The most context a session may end with and still be continued, in
/// tokens. Measured on a product run: a light `code` session ends near
/// 40–90k, a heavy one past 200k.
pub const CHAIN_CONTEXT_MAX: u64 = 100_000;

/// What a relay is kept under: the milestone, the task's system and the
/// model its sessions open on — a branch on another model reads no cache.
#[must_use]
pub fn relay_key(ctx: &Context<Loop>) -> Option<String> {
    let system = sections::parse(&ctx.state.task.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
        .map(|declaration| declaration.system.trim().to_string())
        .filter(|system| !system.is_empty())?;
    let model = ctx
        .session_spec
        .as_ref()
        .map_or("default", |spec| spec.model.as_str());
    Some(format!("{}-{system}-{model}", ctx.state.milestone.number))
}

/// Takes the relay a light session of the same system left, and makes it
/// the base this round's sessions branch from.
pub struct TakeRelay {
    /// The register of relays.
    pub handoffs: Rc<dyn Handoffs>,
}

#[async_trait(?Send)]
impl Action<Loop> for TakeRelay {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || ctx.state.resumed || ctx.base.is_some() {
            return Ok(Verdict::Continue);
        }
        let Some(key) = relay_key(ctx) else {
            return Ok(Verdict::Continue);
        };
        let Some(relay) = self.handoffs.take(&key)? else {
            return Ok(Verdict::Continue);
        };
        if relay.task == ctx.state.task.number {
            return Ok(Verdict::Continue);
        }
        ctx.traces.say(&format!(
            "chain: #{} continues #{}'s session ({}k of context)",
            ctx.state.task.number,
            relay.task,
            relay.context_tokens / 1000
        ));
        ctx.base = Some(Base {
            session: relay.session,
            cost_usd: relay.cost_usd,
        });
        Ok(Verdict::Continue)
    }
}

/// After `code`: leaves the session for the next task of the system when
/// it ended under [`CHAIN_CONTEXT_MAX`].
pub struct LeaveRelay {
    /// The register of relays.
    pub handoffs: Rc<dyn Handoffs>,
}

#[async_trait(?Send)]
impl Action<Loop> for LeaveRelay {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get("code") else {
            return Ok(Verdict::Continue);
        };
        let Some(session) = reply.spend.session.clone() else {
            return Ok(Verdict::Continue);
        };
        let tokens = &reply.spend.tokens;
        let context = tokens.input.unwrap_or(0) + tokens.cache_write.unwrap_or(0);
        let cost_usd = reply.spend.cost_usd;
        if context == 0 || context >= CHAIN_CONTEXT_MAX {
            return Ok(Verdict::Continue);
        }
        let Some(key) = relay_key(ctx) else {
            return Ok(Verdict::Continue);
        };
        self.handoffs.leave(
            &key,
            &Handoff {
                session,
                cost_usd,
                task: ctx.state.task.number.clone(),
                context_tokens: context,
            },
        )?;
        ctx.traces.say(&format!(
            "chain: #{}'s session ({}k of context) is left for the next task of its system",
            ctx.state.task.number,
            context / 1000
        ));
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_handoffs::FakeHandoffs;
    use harness_core::domain::{Named, Spend, Tokens};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn ctx(task: &str, dry_run: bool) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            Loop {
                milestone: Named {
                    number: "18".to_string(),
                    ..Named::default()
                },
                task: Named {
                    number: task.to_string(),
                    title: "a task".to_string(),
                    body: "## Architecture\n\nunit: capability\nsystem: campaign\n".to_string(),
                },
                ..Loop::default()
            },
            Logbook::null(),
        )
    }

    fn coded(ctx: &mut Context<Loop>, session: Option<&str>, written: u64) {
        ctx.results.insert(
            "code".to_string(),
            Reply {
                text: String::new(),
                stop_line: None,
                spend: Spend {
                    session: session.map(ToString::to_string),
                    cost_usd: Some(0.4),
                    tokens: Tokens {
                        input: Some(2_000),
                        cache_write: Some(written),
                        ..Tokens::default()
                    },
                    ..Spend::default()
                },
            },
        );
    }

    #[tokio::test]
    async fn a_light_session_is_left_and_the_next_task_of_its_system_branches_from_it() {
        let register = Rc::new(FakeHandoffs::default());
        let mut done = ctx("45", false);
        coded(&mut done, Some("s-45"), 60_000);
        LeaveRelay {
            handoffs: Rc::clone(&register) as Rc<dyn Handoffs>,
        }
        .run(&mut done)
        .await
        .expect("left");
        assert!(register.left.borrow().contains_key("18-campaign-default"));

        let take = TakeRelay {
            handoffs: Rc::clone(&register) as Rc<dyn Handoffs>,
        };
        let mut next = ctx("46", false);
        take.run(&mut next).await.expect("taken");
        assert_eq!(
            next.base,
            Some(Base {
                session: "s-45".to_string(),
                cost_usd: Some(0.4)
            })
        );
        let mut third = ctx("47", false);
        take.run(&mut third).await.expect("none left");
        assert_eq!(third.base, None, "a relay is taken once");
    }

    #[tokio::test]
    async fn a_heavy_or_idless_session_leaves_nothing() {
        let register = Rc::new(FakeHandoffs::default());
        let leave = LeaveRelay {
            handoffs: Rc::clone(&register) as Rc<dyn Handoffs>,
        };
        let mut heavy = ctx("45", false);
        coded(&mut heavy, Some("s-45"), CHAIN_CONTEXT_MAX);
        leave.run(&mut heavy).await.expect("ran");
        let mut idless = ctx("45", false);
        coded(&mut idless, None, 10_000);
        leave.run(&mut idless).await.expect("ran");
        let mut dry = ctx("45", true);
        coded(&mut dry, Some("s-45"), 10_000);
        leave.run(&mut dry).await.expect("ran");
        assert!(register.left.borrow().is_empty());
    }

    #[tokio::test]
    async fn its_own_relay_a_resumed_task_or_a_dry_run_is_never_branched_from() {
        let register = Rc::new(FakeHandoffs::default());
        let relay = Handoff {
            session: "s-45".to_string(),
            cost_usd: None,
            task: "45".to_string(),
            context_tokens: 50_000,
        };
        let take = TakeRelay {
            handoffs: Rc::clone(&register) as Rc<dyn Handoffs>,
        };
        register.leave("18-campaign-default", &relay).expect("left");
        let mut own = ctx("45", false);
        take.run(&mut own).await.expect("ran");
        assert_eq!(own.base, None, "its own");
        register.leave("18-campaign-default", &relay).expect("left");
        let mut resumed = ctx("46", false);
        resumed.state.resumed = true;
        take.run(&mut resumed).await.expect("ran");
        assert_eq!(resumed.base, None, "resumed");
        let mut dry = ctx("46", true);
        take.run(&mut dry).await.expect("ran");
        assert_eq!(dry.base, None, "dry");
        assert!(
            register.left.borrow().contains_key("18-campaign-default"),
            "left untaken"
        );
    }
}
