//! La boucle entière : ses portes, puis N tours du même round.
//!
//! **Le workflow compte les tours, le round reçoit son numéro** (décision
//! n°13). Le budget est un réglage du run ; un round n'a pas à savoir qu'il est
//! le 3ᵉ de 5, et ne peut pas le savoir — il reçoit un `u32` et rien d'autre.
//!
//! # Pourquoi une boucle écrite, et non une forme `Repeat` du core
//!
//! Le Python avait un `Repeat(unit=…, budget=…, exhausted=…)` dans
//! `core/execution/shapes/`, c'est-à-dire une couche déclarative pour quinze
//! lignes de `for`. Ce dépôt a déjà retiré un moteur de graphe parce que sa
//! couche déclarative mentait sur l'exécution ; en remettre une pour un seul
//! usage serait refaire le trajet dans l'autre sens. Le jour où un second
//! workflow veut la même forme, elle se factorisera — avec deux exemples sous
//! les yeux plutôt qu'un.
//!
//! # Ce que le workflow ne vérifie pas après coup
//!
//! Rien. Ce que la boucle garantit se garantit **par round** — une PR mergée
//! qui porte `Closes #N` —, donc au moment où le round finit, donc avant que le
//! suivant soit payé. Le revérifier ici ne dirait rien de plus et le dirait
//! trop tard.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::store::checkpoint::Checkpoint;
use harness_core::domain::{Halt, Outcome, Resumable, Verdict};
use harness_core::execution::{Context, Executable, Gate, Guarded};

use crate::dev_loop::round::TaskRound;
use crate::dev_loop::state::Loop;

/// La boucle de développement : un préflight, puis des tours.
pub struct DevLoop {
    /// Ce qui doit tenir avant que le premier stage soit payé.
    pub pre: Gate<Loop>,
    /// Combien de tours au plus.
    pub budget: u32,
    /// Ce qui construit le round d'un tour. Une fabrique et non un round
    /// réutilisé : chaque tour a son numéro, qui entre dans la colonne `round`
    /// du registre et dans l'étiquette du journal.
    pub rounds: Box<dyn Fn(u32) -> TaskRound>,
    /// Où le point de reprise est écrit, quand il y en a un.
    pub store: Option<Rc<Checkpoint>>,
    /// L'identifiant du flow, qui nomme le fichier d'états.
    pub flow_id: String,
}

#[async_trait(?Send)]
impl Executable<Loop> for DevLoop {
    fn pre(&self) -> Option<&Gate<Loop>> {
        Some(&self.pre)
    }

    async fn perform(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        for turn in 1..=self.budget {
            if turn > 1 {
                // La task précédente est finie : son `stages_done` part avec
                // elle, sinon les trois stages de la suivante seraient sautés.
                ctx.state = ctx.state.turned();
            }
            let round = (self.rounds)(turn);
            let said = ctx.traces.clone();
            said.say(&format!("--- round {turn}/{} ---", self.budget));
            let verdict = round.execute(ctx).await;
            // Écrit avant de propager : un round qui s'arrête au milieu est
            // justement celui dont la reprise a besoin. Ne pas le faire
            // referait payer les stages déjà passées.
            self.remember(ctx)?;
            if let Verdict::NothingLeft(why) = verdict? {
                said.say(&why);
                return Ok(Verdict::NothingLeft(why));
            }
            // La task est livrée : le point de reprise n'a plus rien à décrire,
            // et le laisser ferait rechoisir une task finie au run suivant.
            self.forget()?;
        }
        Ok(Verdict::Continue)
    }
}

impl DevLoop {
    /// Écrit le pointeur et l'état, s'il y a un magasin.
    fn remember(&self, ctx: &Context<Loop>) -> Outcome<()> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        if ctx.settings.dry_run || !ctx.state.has_task() {
            return Ok(());
        }
        store.set_pointer(&ctx.state.task_key, &self.flow_id)?;
        let state = serde_json::to_value(&ctx.state)
            .map_err(|e| Halt::Failed(format!("l'état du round ne se sérialise pas : {e}")))?;
        let step = ctx.state.done().last().cloned().unwrap_or_default();
        store.save(&self.flow_id, &step, &state)
    }

    /// Efface le point de reprise.
    fn forget(&self) -> Outcome<()> {
        self.store.as_ref().map_or(Ok(()), |store| store.clear())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::common::labels;
    use crate::dev_loop::actions::{MarkWaitingMerge, PickTask};
    use crate::dev_loop::stages;
    use crate::dev_loop::wiring::fake;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::{Logbook, Sink, Verbosity};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    /// Un tableau dont toutes les tasks d'agent sont fermées : chaque round
    /// bascule donc en rollover, et aucun ne paie.
    fn finished_milestone() -> Rc<FakeGitHub> {
        let mut done = issue(11, &[labels::AGENT]);
        done.state = "closed".to_string();
        Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        })
    }

    fn loop_over(gh: &Rc<FakeGitHub>, budget: u32) -> DevLoop {
        let port = Rc::clone(gh);
        DevLoop {
            pre: Gate::empty("préflight"),
            budget,
            rounds: Box::new(move |turn| {
                let wiring = fake::with(Rc::clone(&port));
                let gh = Rc::clone(&wiring.gh);
                TaskRound {
                    turn,
                    pick: PickTask {
                        gh: Rc::clone(&gh),
                        resuming: None,
                    },
                    stages: stages::table(&wiring, turn),
                    rollover: None,
                    delivered: MarkWaitingMerge {
                        gh,
                        integration_branch: wiring.integration_branch.clone(),
                    },
                    post: Gate::empty("livré"),
                }
            }),
            store: None,
            flow_id: "test".to_string(),
        }
    }

    fn ctx(log: Logbook) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            Loop::default(),
            log,
        )
    }

    #[tokio::test]
    async fn nothing_left_stops_the_loop_instead_of_replaying_the_rollover() {
        // Sans ça, un milestone fini sans rollover branché ferait tourner les
        // trois rounds du budget pour réapprendre trois fois qu'il n'y a rien.
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        let mut context = ctx(log);
        let verdict = loop_over(&finished_milestone(), 3)
            .execute(&mut context)
            .await
            .expect("un arrêt propre");
        assert!(matches!(verdict, Verdict::NothingLeft(_)));
        let said = capture.0.borrow().join("\n");
        assert!(said.contains("round 1/3"));
        assert!(
            !said.contains("round 2/3"),
            "le second tour n'a pas eu lieu"
        );
    }

    #[tokio::test]
    async fn a_round_that_halts_stops_the_loop_and_keeps_its_reason() {
        // Une task ouverte mais pas prête : le round s'arrête, et le budget
        // restant ne sert pas à la redemander.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(4, &[labels::MILESTONE])],
            subs: vec![(4, vec![issue(11, &[labels::AGENT])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Logbook::null());
        let err = loop_over(&gh, 3)
            .execute(&mut context)
            .await
            .expect_err("doit s'arrêter");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains(labels::READY));
    }

    #[test]
    fn a_new_turn_forgets_the_stages_of_the_task_that_just_finished() {
        // L'invariant : `stages_done` est « ce qui a tourné pour cette task ».
        // Le garder d'un tour à l'autre sauterait les trois stages suivants.
        let mut first = Loop::default();
        first.milestone.number = "4".to_string();
        first.task_key = "11".to_string();
        first.mark("code");
        let next = first.turned();
        assert_eq!(next.milestone.number, "4", "le milestone reste");
        assert!(next.done().is_empty());
        assert!(!next.has_task());
    }
}
