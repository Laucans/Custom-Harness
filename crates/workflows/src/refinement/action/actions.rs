//! What refinement asks of its sessions, and what it records afterward.
//!
//! [`AskRefine`] receives its template rather than fetching it: texts live
//! with the table that sends them
//! ([`orchestration::prompts`](crate::refinement::orchestration::prompts)), and
//! an action that went to read them would reverse the composition's direction.
//! Same for stage names: they descend as a field from the table, the only place
//! where they are written.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::agent::Base;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::store::spending::Spending;

use crate::common::explore;
use crate::refinement::data::phase::Phase;
use crate::refinement::data::state::RefinementState;
use crate::refinement::data::{rounds, sections};

/// What a human asked for this round, verbatim. Empty when they said nothing.
fn additional_context_block(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!(
        "--- additional_context ---\nWhat the human asked for in this round, \
         verbatim:\n{text}\n--- end additional_context ---"
    )
}

/// What every refinement session hears before its own question: the map
/// and, when the issue sits in a milestone, the hierarchy around it.
fn shared_context(
    open: &Open<'_, RefinementState>,
    explore: bool,
    artifacts_dir: &Path,
    disk: &dyn Disk,
) -> String {
    let map_file = explore::map_file_path(artifacts_dir);
    let prefix = explore::repo_context(open.ctx, explore, &map_file, disk);
    let hierarchy = &open.state.hierarchy;
    if hierarchy.is_empty() {
        prefix
    } else {
        format!("{prefix}\n\n{hierarchy}")
    }
}

/// Tells the shared context once, in the session every later step branches
/// from.
///
/// Without it, each step re-wrote the same thirty-odd thousand tokens into
/// the prompt cache. With it, a step re-reads them at a tenth of the price
/// and writes only its own question. The carrier reports no session (a
/// dry-run): no base, and each step carries the context itself, as before.
pub struct Prime {
    /// The stage name, for the journal and the registry.
    pub stage: String,
    /// What the base is asked once it has the context, received from the table.
    pub ask: String,
    /// This issue's artifacts folder.
    pub artifacts_dir: PathBuf,
    /// `--explore`.
    pub explore: bool,
    /// What reads a kept map back.
    pub disk: Rc<dyn Disk>,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<RefinementState> for Prime {
    async fn run(&self, open: &mut Open<'_, RefinementState>) -> Outcome<Verdict> {
        let context = shared_context(open, self.explore, &self.artifacts_dir, self.disk.as_ref());
        let prompt = format!("{context}\n\n{}", self.ask);
        let round = open.state.round_no;
        let task = format!("#{}", open.state.issue().number);
        let reply = ask_and_record(
            open,
            &prompt,
            &self.stage,
            round,
            &task,
            self.spending.as_ref(),
        )
        .await?;
        open.ctx.base = reply.spend.session.map(|session| Base {
            session,
            cost_usd: reply.spend.cost_usd,
        });
        Ok(Verdict::Continue)
    }
}

/// Sends the prompt for a paid refinement step.
///
/// One type for every paid step — router, sections, coherence, advice — because they
/// compose their prompt the same way: the same template, the same scope,
/// prefixed by the repository map — unless the step branched from the
/// [`Prime`] base, which told it already.
pub struct AskRefine {
    /// The stage name, for the journal and the `stage` column of the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: String,
    /// Does this step read the body this round is about to publish, rather
    /// than the one before this round?
    ///
    /// True for coherence and the advice: they read together sections that
    /// no one else, in this round, has read together.
    pub merged_body: bool,
    /// Which half of the refinement this step belongs to — the keys it
    /// names to the model are this phase's.
    pub phase: Phase,
    /// What a human asked for this round, verbatim.
    pub context: String,
    /// This issue's artifacts folder.
    pub artifacts_dir: PathBuf,
    /// `--explore`.
    pub explore: bool,
    /// What reads a kept map back.
    pub disk: Rc<dyn Disk>,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<RefinementState> for AskRefine {
    async fn run(&self, open: &mut Open<'_, RefinementState>) -> Outcome<Verdict> {
        let issue = open.state.issue().clone();
        let body = if self.merged_body {
            sections::render(&sections::merge(
                &open.state.found,
                &open.state.wanted,
                &open.ctx.results,
            ))
        } else {
            let trimmed = issue.body.trim();
            if trimmed.is_empty() {
                harness_core::domain::prompts::EMPTY_BODY.to_string()
            } else {
                trimmed.to_string()
            }
        };
        let filled = splice(
            &self.template,
            &[
                ("num", &issue.number.to_string()),
                ("title", &issue.title),
                ("round", &open.state.round_no.to_string()),
                ("keys", &self.phase.keys().join("\n")),
                ("drags", self.phase.drags()),
                (
                    "additional_context",
                    &additional_context_block(&self.context),
                ),
                ("body", &body),
            ],
        );
        // A step branched from the base already holds the shared context.
        let prompt = if open.ctx.base.is_some() {
            filled
        } else {
            let context =
                shared_context(open, self.explore, &self.artifacts_dir, self.disk.as_ref());
            format!("{context}\n\n{filled}")
        };
        let round = open.state.round_no;
        let task = format!("#{}", issue.number);
        ask_and_record(
            open,
            &prompt,
            &self.stage,
            round,
            &task,
            self.spending.as_ref(),
        )
        .await?;
        Ok(Verdict::Continue)
    }
}

/// The "write" half of the former `router_named_sections`: sets
/// `state.wanted` from the router's reply.
///
/// A local action, slipped in after the session — the same split as
/// `dev_loop::action::actions`.
pub struct RecordWantedSections {
    /// The router's stage name, under which its reply is stored in
    /// `ctx.results`. Received from the table, never hard-coded here.
    pub router: String,
}

#[async_trait(?Send)]
impl Action<RefinementState> for RecordWantedSections {
    async fn run(&self, ctx: &mut Context<RefinementState>) -> Outcome<Verdict> {
        if let Some(reply) = ctx.results.get(&self.router) {
            ctx.state.wanted = rounds::wanted_from(&reply.text, ctx.state.phase);
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::ports::agent::{Reply, Session};
    use harness_core::traces::Logbook;

    use crate::common::fake_disk::FakeDisk;
    use crate::refinement::ports::fake::Nowhere;

    /// A session that keeps what it was told and reports this session id.
    struct Heard {
        prompts: Vec<String>,
        session: Option<String>,
    }

    #[async_trait(?Send)]
    impl Session for Heard {
        async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
            self.prompts.push(prompt.to_string());
            Ok(Reply {
                text: "OK".to_string(),
                stop_line: None,
                spend: Spend {
                    cost_usd: Some(0.14),
                    session: self.session.clone(),
                    ..Spend::default()
                },
            })
        }
    }

    fn heard(session: Option<&str>) -> Heard {
        Heard {
            prompts: Vec::new(),
            session: session.map(ToString::to_string),
        }
    }

    /// A context on issue #25, inside a milestone. Dry-run keeps the
    /// ledger out; the map then reads as the dry-run one.
    fn context() -> Context<RefinementState> {
        Context::new(
            Settings {
                dry_run: true,
                stages: String::new(),
            },
            RefinementState {
                issue: Some(Issue {
                    number: 25,
                    title: "Schema".to_string(),
                    body: "the body".to_string(),
                    ..Issue::default()
                }),
                hierarchy: "--- HIERARCHY ---".to_string(),
                ..RefinementState::default()
            },
            Logbook::null(),
        )
    }

    fn prime() -> Prime {
        Prime {
            stage: "prime".to_string(),
            ask: "reply OK".to_string(),
            artifacts_dir: PathBuf::from("/tmp/refinement/25"),
            explore: false,
            disk: Rc::new(FakeDisk::default()),
            spending: Rc::new(Nowhere),
        }
    }

    fn ask() -> AskRefine {
        AskRefine {
            stage: "business-goal".to_string(),
            template: "write #{num}".to_string(),
            merged_body: false,
            phase: Phase::Business,
            context: String::new(),
            artifacts_dir: PathBuf::from("/tmp/refinement/25"),
            explore: false,
            disk: Rc::new(FakeDisk::default()),
            spending: Rc::new(Nowhere),
        }
    }

    #[tokio::test]
    async fn the_base_hears_the_map_and_the_hierarchy_then_becomes_the_round_base() {
        let mut ctx = context();
        let mut session = heard(Some("base-1"));
        let mut open = Open {
            ctx: &mut ctx,
            session: &mut session,
        };
        prime().run(&mut open).await.unwrap();
        let said = &session.prompts[0];
        assert!(said.contains("REPO MAP"));
        assert!(said.contains("--- HIERARCHY ---"));
        assert!(said.ends_with("reply OK"));
        assert_eq!(
            ctx.base,
            Some(Base {
                session: "base-1".to_string(),
                cost_usd: Some(0.14),
            })
        );
    }

    #[tokio::test]
    async fn a_carrier_that_reports_no_session_leaves_no_base() {
        let mut ctx = context();
        let mut session = heard(None);
        let mut open = Open {
            ctx: &mut ctx,
            session: &mut session,
        };
        prime().run(&mut open).await.unwrap();
        assert_eq!(ctx.base, None);
    }

    #[tokio::test]
    async fn a_step_without_a_base_carries_the_shared_context_itself() {
        let mut ctx = context();
        let mut session = heard(Some("fresh"));
        let mut open = Open {
            ctx: &mut ctx,
            session: &mut session,
        };
        ask().run(&mut open).await.unwrap();
        let said = &session.prompts[0];
        assert!(said.starts_with("--- REPO MAP"));
        assert!(said.contains("--- HIERARCHY ---\n\nwrite #25"));
    }

    #[tokio::test]
    async fn a_step_branched_from_the_base_sends_only_its_own_question() {
        let mut ctx = context();
        ctx.base = Some(Base {
            session: "base-1".to_string(),
            cost_usd: None,
        });
        let mut session = heard(Some("branch"));
        let mut open = Open {
            ctx: &mut ctx,
            session: &mut session,
        };
        ask().run(&mut open).await.unwrap();
        assert_eq!(session.prompts, ["write #25"]);
    }
}
