//! What the paid stage requires before it costs anything, and what it must
//! obtain.

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::contract_manager::data::change;
use crate::contract_manager::data::state::ManagerState;

/// Someone else must produce the id: one the graph does not know is the
/// requester's own, and nothing is paid.
pub struct NeedsASession;

#[async_trait(?Send)]
impl Verification<ManagerState> for NeedsASession {
    fn purpose(&self) -> String {
        "the plan's graph knows the id: another milestone produces it".to_string()
    }

    async fn verify(&self, ctx: &Context<ManagerState>) -> Outcome<Verdict> {
        Ok(if ctx.state.nobody_else_s() {
            Verdict::Skip(format!(
                "`{}` is no milestone's contract but the requester's own",
                ctx.state.request().id
            ))
        } else {
            Verdict::Continue
        })
    }
}

/// The session's reply reads as a new version of this contract.
pub struct ChangeParses {
    /// The stage whose reply this judges.
    pub stage: String,
}

#[async_trait(?Send)]
impl Verification<ManagerState> for ChangeParses {
    fn purpose(&self) -> String {
        "the reply reads as an answer: the contract evolved (its manifest at a higher version), \
         duplicated (a new id in the requester's system, at version 1), or handed back with a \
         reason"
            .to_string()
    }

    async fn verify(&self, ctx: &Context<ManagerState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || ctx.state.nobody_else_s() {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get(&self.stage) else {
            return Err(Halt::Failed(format!(
                "no reply recorded under {} — the evolve stage did not run",
                self.stage
            )));
        };
        change::parse(&reply.text, &ctx.state.request().id, ctx.state.version())
            .map(|_| Verdict::Continue)
            .map_err(|why| {
                Halt::Failed(format!(
                    "the change did not read, even after one retry: {why}"
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_manager::data::request::Request;
    use harness_architecture::graph::Kind;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::Reply;
    use harness_core::traces::Logbook;

    fn ctx(answer: Option<&str>, known: Option<(Kind, u32)>) -> Context<ManagerState> {
        let mut ctx = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            ManagerState {
                issue: Some(Issue::default()),
                request: Some(Request {
                    id: "a.b".to_string(),
                    task: 3,
                    ..Request::default()
                }),
                known,
                ..ManagerState::default()
            },
            Logbook::null(),
        );
        if let Some(text) = answer {
            ctx.results.insert(
                "evolve".to_string(),
                Reply {
                    text: text.to_string(),
                    stop_line: None,
                    spend: Spend::default(),
                },
            );
        }
        ctx
    }

    #[tokio::test]
    async fn an_unknown_id_skips_the_session_and_a_known_one_needs_it() {
        let skipped = NeedsASession
            .verify(&ctx(None, None))
            .await
            .expect("a verdict");
        assert!(matches!(skipped, Verdict::Skip(ref why) if why.contains("a.b")));
        let needed = NeedsASession
            .verify(&ctx(None, Some((Kind::Capability, 1))))
            .await
            .expect("a verdict");
        assert_eq!(needed, Verdict::Continue);
        assert!(NeedsASession.purpose().contains("graph"));
    }

    #[tokio::test]
    async fn the_gate_judges_the_reply_against_the_id_and_its_version() {
        let gate = ChangeParses {
            stage: "evolve".to_string(),
        };
        let good = r#"{"path":"x/capability.json","manifest":{"capability":"a.b","version":2},"summary":"s"}"#;
        assert_eq!(
            gate.verify(&ctx(Some(good), Some((Kind::Capability, 1))))
                .await
                .expect("a verdict"),
            Verdict::Continue
        );
        let stale = gate
            .verify(&ctx(Some(good), Some((Kind::Capability, 2))))
            .await
            .expect_err("version did not go up");
        assert!(stale.reason().contains("must go up"));
        let none = gate
            .verify(&ctx(None, Some((Kind::Capability, 1))))
            .await
            .expect_err("no reply");
        assert!(none.reason().contains("did not run"));
        assert_eq!(
            gate.verify(&ctx(None, None)).await.expect("skipped id"),
            Verdict::Continue
        );
        assert!(gate.purpose().contains("higher version"));
        let back = r#"{"answer":"hand_back","reason":"nothing to change"}"#;
        assert_eq!(
            gate.verify(&ctx(Some(back), Some((Kind::Capability, 1))))
                .await
                .expect("a verdict"),
            Verdict::Continue
        );
    }
}
