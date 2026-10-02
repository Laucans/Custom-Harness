//! Un round : choisir la task, faire tourner la séquence, constater la
//! livraison.
//!
//! **La séquence n'est pas ici.** Elle est dans `stages.rs`, une entrée par
//! stage. Ce module porte ce qui l'entoure : la branche de rollover, et la
//! post-condition qui suit.
//!
//! Un type écrit à la main plutôt qu'un [`Round`](harness_core::execution::Round)
//! générique — la variante B de `docs/ROUND-DRAFT.md`. La raison est que ce
//! round **bifurque** : une table déclarative devrait porter un routeur pour le
//! dire, et le dépôt a déjà retiré un moteur de graphe exactement pour ça. Ce
//! qui reste du graphe est un `if/else` :
//!
//! ```text
//! PickTask
//!   ├── pas de task, aucune ouverte ..... rollover : /planner
//!   ├── pas de task, des ouvertes ....... arrêt : dire quel geste débloque
//!   └── une task ........................ la séquence, puis la livraison
//! ```
//!
//! Le routeur du moteur avait trois sorties dont une muette, parce que c'était
//! la façon la plus courte de dire « ce round ne va nulle part » à un moteur
//! qui, sinon, enchaînait. Un `Err(Halt)` et un `Verdict::NothingLeft` le
//! disent sans routeur.

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context, Executable, Gate, Guarded, Stage};

use crate::dev_loop::actions::{MarkWaitingMerge, PickTask};
use crate::dev_loop::state::Loop;

/// Un round de la boucle de développement.
pub struct TaskRound {
    /// Le numéro du tour. Reçu, jamais compté ici : c'est le workflow qui
    /// compte les tours (décision n°13), et un round n'a pas à savoir qu'il est
    /// le 3ᵉ de 5.
    pub turn: u32,
    /// Ce qui choisit la task, ou bascule en rollover.
    pub pick: PickTask,
    /// La séquence, dans l'ordre. Vient de `stages::table`.
    pub stages: Vec<Stage<Loop>>,
    /// Le stage de rollover, s'il est branché.
    ///
    /// `None` est le défaut, et c'est un choix : enchaîner en non surveillé
    /// dépense un run opus et engage le projet sur un item de roadmap que
    /// personne n'a lu.
    pub rollover: Option<Stage<Loop>>,
    /// Ce qui marque la task livrée quand une PR mergée le prouve.
    pub delivered: MarkWaitingMerge,
    /// Ce que le round doit avoir obtenu.
    pub post: Gate<Loop>,
}

#[async_trait(?Send)]
impl Executable<Loop> for TaskRound {
    fn post(&self) -> Option<&Gate<Loop>> {
        Some(&self.post)
    }

    async fn perform(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        self.pick.run(ctx).await?;
        if ctx.state.rollover {
            return self.roll(ctx).await;
        }
        for stage in &self.stages {
            stage.execute(ctx).await?;
        }
        // La dernière chose qu'un round fait : marquer. La garde qui suit juge.
        self.delivered.run(ctx).await
    }
}

impl TaskRound {
    /// La branche de rollover : ouvrir l'item de roadmap suivant, ou s'arrêter.
    async fn roll(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        let Some(planner) = &self.rollover else {
            // `NothingLeft` et non un succès : le workflow doit cesser de
            // relancer des rounds, pas en payer un de plus pour réapprendre
            // qu'il n'y a rien.
            return Ok(Verdict::NothingLeft(
                "no rollover stage wired — the milestone is finished and \
                 nothing is set to open the next roadmap item"
                    .to_string(),
            ));
        };
        planner.execute(ctx).await?;
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::common::labels;
    use crate::dev_loop::wiring::fake;
    use crate::dev_loop::{gates, stages};
    use harness_core::domain::{Halt, Issue, Resumable};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use std::rc::Rc;

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn ctx(stages: &str, dry_run: bool) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run,
                stages: stages.to_string(),
            },
            Loop::default(),
            Logbook::null(),
        )
    }

    /// Un round monté contre ce GitHub-là, rollover branché ou non.
    fn round(gh: &Rc<FakeGitHub>, with_rollover: bool) -> TaskRound {
        let wiring = fake::with(Rc::clone(gh));
        let port = Rc::clone(&wiring.gh);
        TaskRound {
            turn: 1,
            pick: PickTask {
                gh: Rc::clone(&port),
                resuming: None,
            },
            stages: stages::table(&wiring, 1),
            rollover: with_rollover.then(|| stages::planner(&wiring, 1)),
            delivered: MarkWaitingMerge {
                gh: Rc::clone(&port),
                integration_branch: wiring.integration_branch.clone(),
            },
            post: Gate {
                name: "le round doit avoir livré",
                checks: vec![Box::new(gates::AMergedPrClosesTheTask {
                    gh: port,
                    integration_branch: wiring.integration_branch.clone(),
                    code_runs: true,
                    stages: String::new(),
                })],
            },
        }
    }

    #[tokio::test]
    async fn a_dry_run_names_the_task_it_would_pick_and_opens_nothing() {
        // Le câblage de test refuse d'ouvrir une session : si une seule
        // s'ouvrait, ce test échouerait sur `Halt::Failed`.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![issue(11, &[labels::AGENT, labels::READY])])],
            ..FakeGitHub::default()
        });
        // `--stages` vide ferait tourner les trois stages ; un dry-run réel
        // câble une fabrique de répétition. Ici on vérifie le choix seul.
        let mut context = ctx("rien-de-connu", true);
        let verdict = round(&gh, false)
            .execute(&mut context)
            .await
            .expect("un round à blanc");
        assert_eq!(verdict, Verdict::Continue);
        assert_eq!(context.state.task_key, "11");
        assert!(gh.writes().is_empty(), "un dry-run n'écrit rien");
        assert!(context.state.done().is_empty(), "et ne marque rien");
    }

    #[tokio::test]
    async fn a_finished_milestone_without_a_rollover_says_there_is_nothing_left() {
        let mut done = issue(11, &[labels::AGENT]);
        done.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("", false);
        let verdict = round(&gh, false)
            .execute(&mut context)
            .await
            .expect("un rollover");
        let Verdict::NothingLeft(why) = verdict else {
            panic!("le workflow doit cesser de relancer, pas enchaîner");
        };
        assert!(why.contains("no rollover stage wired"));
        assert!(context.state.rollover);
    }

    #[tokio::test]
    async fn a_rollover_round_passes_the_delivery_gate_having_no_task() {
        // Le mode de panne que ça évite : la post-condition du round réclame
        // une PR mergée pour une task qui n'existe pas, et le rollover ne peut
        // jamais aboutir.
        let mut done = issue(11, &[labels::AGENT]);
        done.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("", false);
        assert!(round(&gh, false).execute(&mut context).await.is_ok());
        assert!(!context.state.has_task());
    }

    #[tokio::test]
    async fn open_but_unplayable_tasks_stop_the_round_before_any_stage() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![issue(11, &[labels::AGENT])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("", false);
        let err = round(&gh, false)
            .execute(&mut context)
            .await
            .expect_err("doit s'arrêter");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(!context.state.rollover, "ce n'est pas un rollover");
    }

    #[tokio::test]
    async fn a_round_whose_every_stage_is_filtered_out_still_demands_the_proof() {
        // Le cas réel : `--stages business-analyst` ne livre rien, donc rien ne
        // marque la task. Sans la post-condition, le round suivant la
        // rechoisirait et repayerait la rédaction du même SPEC.
        let gh = Rc::new(FakeGitHub {
            issues: vec![
                issue(4, &[labels::MILESTONE]),
                issue(11, &[labels::AGENT, labels::READY]),
            ],
            subs: vec![(4, vec![issue(11, &[labels::AGENT, labels::READY])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx("aucun-stage-connu", false);
        let err = round(&gh, false)
            .execute(&mut context)
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains("Closes #11"));
        assert!(err.reason().contains("Stopping rather than looping"));
    }

    #[tokio::test]
    async fn a_task_a_merged_pr_closes_is_marked_waiting_merge_not_closed() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![
                issue(4, &[labels::MILESTONE]),
                issue(11, &[labels::AGENT, labels::READY]),
            ],
            subs: vec![(4, vec![issue(11, &[labels::AGENT, labels::READY])])],
            merged: vec![Issue {
                number: 99,
                body: "Closes #11".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        let mut context = ctx("aucun-stage-connu", false);
        round(&gh, false)
            .execute(&mut context)
            .await
            .expect("un round livré");
        assert_eq!(
            gh.writes(),
            vec![Wrote::Label(11, labels::WAITING_MERGE.to_string())]
        );
    }
}
