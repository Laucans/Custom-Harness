//! Ce qu'une stage du round exige avant de payer, et ce qu'elle doit obtenir.
//!
//! À ne pas confondre avec les gates du **workflow**, vérifiées une fois avant
//! et après le run. Celles-ci n'ont de sens que dans le round.
//!
//! # Les deux hybrides, scindées
//!
//! La décision n°1 dit qu'une `Verification` juge et n'écrit pas — et le borrow
//! checker le tient. Or deux gardes du Python **écrivaient** :
//!
//! | Python | ce qu'elle faisait | devient ici |
//! | --- | --- | --- |
//! | `spec_is_in_the_issue` | relit, pose `spec-written`, mute l'état | [`IssueBodyIsNotEmpty`] + `RecordSpecWritten` |
//! | `task_is_delivered` | relit, pose `waiting-merge` | [`AMergedPrClosesTheTask`] + `MarkWaitingMerge` |
//!
//! Les actions vivent dans `actions.rs` : c'est le sens de la scission.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::common::labels;
use crate::dev_loop::state::Loop;
use crate::dev_loop::{board, tasks};

/// L'étiquette dit que le SPEC est déjà écrit : on reprend après, on ne repaie
/// pas une seconde rédaction par-dessus la première.
pub struct SpecAlreadyWritten;

#[async_trait(?Send)]
impl Verification<Loop> for SpecAlreadyWritten {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if !ctx.state.spec_written {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "issue #{} already carries {} — skipping /business-analyst \
             (resuming a previous run)",
            ctx.state.task.number,
            labels::SPEC_WRITTEN
        )))
    }
}

/// La portée du stage `code` : sans corps d'issue, il n'y a pas de SPEC.
pub struct CodeHasASpec;

#[async_trait(?Send)]
impl Verification<Loop> for CodeHasASpec {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.task.body.trim().is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "issue #{} has an empty body — there is no SPEC to build from. Run \
             /business-analyst on it first (--stages business-analyst).",
            ctx.state.task.number
        )))
    }
}

/// La PR de ce `/code` a-t-elle déjà mergé ? — on ne la repaie pas.
///
/// **Seulement sur un round repris.** Sur un round neuf, `/code` n'a jamais
/// tourné, et la recherche de PR coûterait un appel d'API par round pour une
/// réponse connue d'avance.
///
/// La même preuve qu'exige [`AMergedPrClosesTheTask`], posée *avant* de payer
/// plutôt qu'après : c'est la seule des gardes qui dise « c'est déjà fait »
/// plutôt que « ça ne va pas ».
pub struct CodeAlreadyDelivered {
    /// De quoi lire l'issue et les PR.
    pub gh: Rc<dyn GitHub>,
    /// La branche sur laquelle la preuve est cherchée.
    pub integration_branch: String,
    /// `--restart` rejoue le stage même si la preuve est là.
    pub restart: bool,
}

#[async_trait(?Send)]
impl Verification<Loop> for CodeAlreadyDelivered {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if !ctx.state.resumed || ctx.settings.dry_run || self.restart {
            return Ok(Verdict::Continue);
        }
        let number: u64 = ctx.state.task.number.parse().map_err(|_| {
            Halt::Failed(format!(
                "numéro de task illisible : {:?}",
                ctx.state.task.number
            ))
        })?;
        let here = self.gh.issue(number).await?;
        let shipped = here.is_closed()
            || tasks::waiting_merge(&here)
            || tasks::first_closing(&self.gh.merged_prs(&self.integration_branch).await?, number)
                .is_some();
        if !shipped {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "#{number} est déjà livrée sur {} — /code saute plutôt que d'être \
             repayé (--restart pour le rejouer)",
            self.integration_branch
        )))
    }
}

/// Le corps de l'issue n'est pas vide — la moitié « juge » de l'ex-
/// `spec_is_in_the_issue`.
///
/// Relu chez GitHub, pas supposé : le corps de l'issue **est** le SPEC, et
/// c'est ce que le stage suivant recevra dans sa portée.
pub struct IssueBodyIsNotEmpty {
    /// De quoi relire l'issue.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Verification<Loop> for IssueBodyIsNotEmpty {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let number: u64 = ctx.state.task.number.parse().map_err(|_| {
            Halt::Failed(format!(
                "numéro de task illisible : {:?}",
                ctx.state.task.number
            ))
        })?;
        let issue = self.gh.issue(number).await?;
        if issue.body.trim().is_empty() {
            return Err(Halt::Halted(format!(
                "/business-analyst left issue #{number} with an empty body — \
                 the SPEC goes there, and /code reads nothing else"
            )));
        }
        Ok(Verdict::Continue)
    }
}

/// Une PR mergée porte-t-elle `Closes #N` — la moitié « juge » de l'ex-
/// `task_is_delivered`.
///
/// Ce qu'on exige n'est pas qu'un stage l'affirme, mais qu'une PR **mergée** le
/// déclare. Sans ce constat, un run `--stages business-analyst` repayerait
/// indéfiniment la rédaction du même SPEC.
pub struct AMergedPrClosesTheTask {
    /// De quoi lire l'issue et les PR.
    pub gh: Rc<dyn GitHub>,
    /// La branche sur laquelle la preuve est cherchée.
    pub integration_branch: String,
    /// Vrai si `--stages` laisse `code` de côté — change le message, pas la
    /// règle.
    pub code_runs: bool,
    /// Ce que `--stages` valait, pour le dire dans le message.
    pub stages: String,
}

#[async_trait(?Send)]
impl Verification<Loop> for AMergedPrClosesTheTask {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.has_task() {
            return Ok(Verdict::Continue);
        }
        let number: u64 = ctx.state.task.number.parse().map_err(|_| {
            Halt::Failed(format!(
                "numéro de task illisible : {:?}",
                ctx.state.task.number
            ))
        })?;
        // Déjà fermée, ou déjà marquée : la preuve est là, rien à reposer.
        let here = self.gh.issue(number).await?;
        if here.is_closed() || tasks::waiting_merge(&here) {
            return Ok(Verdict::Continue);
        }
        let merged = self.gh.merged_prs(&self.integration_branch).await?;
        if tasks::first_closing(&merged, number).is_some() {
            return Ok(Verdict::Continue);
        }
        let left_out = if self.code_runs {
            String::new()
        } else {
            format!(
                " --stages ({}) left /code out, and nothing else delivers a \
                 task.",
                self.stages
            )
        };
        Err(Halt::Halted(format!(
            "issue #{number} ended the round and nothing marks it delivered — \
             no merged PR on {} carries `Closes #{number}` on its own \
             line.{left_out} Stopping rather than looping on the same task.",
            self.integration_branch
        )))
    }
}

/// Le rollover a ouvert de quoi travailler, sur le milestone d'après.
///
/// Relu depuis zéro : le milestone en cours n'est peut-être plus le même. Il ne
/// le sera que si `/planner` a fermé celui qu'il vient de terminer — la boucle
/// travaille sur le milestone ouvert de plus petit numéro, donc un ancien
/// laissé ouvert ferait rouler tous les rounds suivants à vide.
pub struct PlannerOpenedATask {
    /// De quoi relire le tableau.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Verification<Loop> for PlannerOpenedATask {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let after = board::read(self.gh.as_ref()).await?;
        if after.open_agents().is_empty() {
            return Err(Halt::Halted(format!(
                "/planner left nothing to pick up: milestone {} still has no \
                 open {} issue. Either it opened none, or it did not close \
                 milestone #{} — the harness reads the lowest-numbered open \
                 milestone, and that one is still it.",
                after.milestone.reference(),
                labels::AGENT,
                ctx.state.milestone.number
            )));
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::{Issue, Named};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(state: Loop) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            state,
            Logbook::null(),
        )
    }

    fn with_task(number: &str, body: &str) -> Loop {
        Loop {
            task: Named {
                number: number.to_string(),
                title: "une task".to_string(),
                body: body.to_string(),
            },
            task_key: number.to_string(),
            ..Loop::default()
        }
    }

    fn issue(number: u64, labels: &[&str], body: &str) -> Issue {
        Issue {
            number,
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_spec_already_written_skips_rather_than_paying_twice() {
        let mut state = with_task("34", "le SPEC");
        state.spec_written = true;
        let verdict = SpecAlreadyWritten
            .verify(&ctx(state))
            .await
            .expect("un verdict");
        assert!(matches!(verdict, Verdict::Skip(_)));
    }

    #[tokio::test]
    async fn without_the_label_business_analyst_runs() {
        let verdict = SpecAlreadyWritten
            .verify(&ctx(with_task("34", "")))
            .await
            .expect("un verdict");
        assert_eq!(verdict, Verdict::Continue);
    }

    #[tokio::test]
    async fn code_refuses_to_improvise_on_an_empty_spec() {
        let err = CodeHasASpec
            .verify(&ctx(with_task("34", "   ")))
            .await
            .expect_err("doit s'arrêter");
        let Halt::Halted(said) = err else {
            panic!("un arrêt volontaire");
        };
        assert!(said.contains("no SPEC to build from"));
    }

    #[tokio::test]
    async fn a_dry_run_lets_code_through_without_a_spec() {
        let mut context = ctx(with_task("34", ""));
        context.settings.dry_run = true;
        assert_eq!(
            CodeHasASpec.verify(&context).await.expect("un verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_fresh_round_never_looks_for_a_merged_pr_of_its_own_code() {
        // Un appel d'API par round pour une réponse connue d'avance : /code
        // n'a jamais tourné sur un round neuf.
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("ne doit pas être appelé".to_string())),
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: false,
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "le SPEC")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_resumed_round_whose_code_already_merged_skips_it() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            merged: vec![Issue {
                number: 99,
                body: "Closes #34".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: false,
        };
        let mut state = with_task("34", "le SPEC");
        state.resumed = true;
        let Verdict::Skip(why) = gate.verify(&ctx(state)).await.expect("verdict") else {
            panic!("un saut");
        };
        assert!(why.contains("--restart"), "dire comment le rejouer");
    }

    #[tokio::test]
    async fn restart_replays_code_even_when_the_proof_is_there() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[labels::WAITING_MERGE], "")],
            ..FakeGitHub::default()
        });
        let gate = CodeAlreadyDelivered {
            gh,
            integration_branch: "main_agent".to_string(),
            restart: true,
        };
        let mut state = with_task("34", "le SPEC");
        state.resumed = true;
        assert_eq!(
            gate.verify(&ctx(state)).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn the_spec_is_reread_from_github_not_assumed() {
        // L'issue porte un corps vide chez GitHub, même si l'état local dit
        // autre chose : c'est GitHub qui a raison, c'est ce que /code lira.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            ..FakeGitHub::default()
        });
        let gate = IssueBodyIsNotEmpty { gh };
        let err = gate
            .verify(&ctx(with_task("34", "un corps local trompeur")))
            .await
            .expect_err("doit s'arrêter");
        assert!(matches!(err, Halt::Halted(_)));
    }

    #[tokio::test]
    async fn a_non_empty_body_satisfies_the_verification_without_writing() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "le SPEC")],
            ..FakeGitHub::default()
        });
        let gate = IssueBodyIsNotEmpty { gh: gh.clone() };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
        // La moitié « juge » ne pose aucune étiquette : c'est l'action qui le
        // fera, et le borrow checker l'y oblige.
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn a_merged_pr_that_closes_the_task_is_the_proof() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            merged: vec![Issue {
                number: 99,
                body: "Closes #34".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn nothing_marking_delivery_stops_rather_than_looping() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        let err = gate
            .verify(&ctx(with_task("34", "")))
            .await
            .expect_err("doit s'arrêter");
        let Halt::Halted(said) = err else {
            panic!("un arrêt volontaire");
        };
        assert!(said.contains("Closes #34"));
        assert!(said.contains("Stopping rather than looping"));
    }

    #[tokio::test]
    async fn leaving_code_out_of_stages_is_named_in_the_message() {
        // Sans ça, « rien ne marque la livraison » se lit comme un bug alors
        // que c'est `--stages` qui a retiré le seul stage qui livre.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: false,
            stages: "business-analyst".to_string(),
        };
        let err = gate
            .verify(&ctx(with_task("34", "")))
            .await
            .expect_err("doit s'arrêter");
        assert!(format!("{err}").contains("left /code out"));
    }

    #[tokio::test]
    async fn an_already_marked_task_needs_no_further_proof() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(34, &[labels::WAITING_MERGE], "")],
            ..FakeGitHub::default()
        });
        let gate = AMergedPrClosesTheTask {
            gh,
            integration_branch: "main_agent".to_string(),
            code_runs: true,
            stages: String::new(),
        };
        assert_eq!(
            gate.verify(&ctx(with_task("34", "")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_planner_that_opened_nothing_stops_and_says_which_of_two_causes() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE], "")],
            subs: vec![(4, vec![])],
            ..FakeGitHub::default()
        });
        let gate = PlannerOpenedATask { gh };
        let mut state = Loop::default();
        state.milestone.number = "4".to_string();
        let err = gate.verify(&ctx(state)).await.expect_err("doit s'arrêter");
        let said = format!("{err}");
        assert!(said.contains("opened none"));
        assert!(said.contains("did not close milestone"));
    }
}
