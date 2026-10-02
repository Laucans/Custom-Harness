//! Parler à la session ouverte d'une stage : envoyer, relire les marqueurs,
//! consigner la dépense, garder la réponse pour la stage suivante.
//!
//! **Commun aux trois workflows**, et écrit ici pour cette raison précise :
//! la boucle de dev composait ceci en ligne dans son unique action de
//! session, parce qu'elle était seule. La revue de PR et le raffinage en ont
//! chacun besoin, à l'identique — seul le prompt et le point de reprise
//! changent — donc c'est ici que ça vit maintenant, et la boucle délègue.

use crate::adapters::agent::Reply;
use crate::adapters::store::spending::{Entry, Spending};
use crate::domain::{Halt, Outcome, Spend, markers};
use crate::execution::action::Open;

/// Envoie `prompt` dans la session ouverte, et fait ce que toute stage
/// payante fait de sa réponse.
///
/// Écrit la ligne de registre **avant** de propager un échec : un stage mort
/// est justement celui dont on veut la trace, et une ligne à `None` plutôt
/// qu'à zéro — un coût non observé ne doit jamais se relire comme une
/// session gratuite.
///
/// Garde la réponse dans `ctx.results[stage]` pour qu'une stage plus loin
/// dans le même passage puisse la lire — c'est ainsi que la passe « brief »
/// de la revue lit celle de la passe « inline », et que la cohérence du
/// raffinage lit ses cinq sections.
///
/// # Errors
///
/// L'échec de la session, après que la ligne de registre a été écrite. Ou
/// [`Halt::Halted`] si la session s'est arrêtée d'elle-même
/// (`AGENT_LOOP_STOP`) — un résultat correct, et la raison est la sienne.
pub async fn ask_and_record<S>(
    open: &mut Open<'_, S>,
    prompt: &str,
    stage: &str,
    round: u32,
    task: &str,
    spending: &dyn Spending,
) -> Outcome<Reply> {
    let log = open.traces.bind(stage);
    let dry_run = open.settings.dry_run;

    let asked = open.session.ask(prompt).await;
    if !dry_run {
        let blind = Spend::default();
        let (spend, outcome) = match &asked {
            Ok(reply) => (&reply.spend, "ok"),
            Err(halt) => (&blind, halt.prefix()),
        };
        spending.record(&Entry {
            round,
            task,
            stage,
            spend,
            outcome,
        })?;
    }
    let reply = asked?;

    if let Some(stop) = markers::stop_line(&reply.text) {
        // Une session qui répond AGENT_LOOP_STOP s'est arrêtée d'elle-même :
        // un résultat correct, et la raison est la sienne.
        return Err(Halt::Halted(format!("{stop} (/{stage})")));
    }
    match markers::ok_line(&reply.text) {
        Some(ok) => log.say(&ok),
        None if !dry_run => log.say(&format!(
            "/{stage} produced no {} marker — relying on the structural checks",
            markers::OK
        )),
        None => {}
    }

    open.ctx.results.insert(stage.to_string(), reply.clone());
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::store::spending::Entry as SpendingEntry;
    use crate::execution::context::{Context, Settings};
    use crate::traces::{Logbook, Sink, Verbosity};
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Scripted(Result<Reply, Halt>);

    #[async_trait(?Send)]
    impl crate::adapters::agent::Session for Scripted {
        async fn ask(&mut self, _prompt: &str) -> Outcome<Reply> {
            self.0.clone()
        }
    }

    #[derive(Default)]
    struct Recorded(RefCell<Vec<(String, String, Option<f64>)>>);

    impl Spending for Recorded {
        fn record(&self, entry: &SpendingEntry<'_>) -> Outcome<()> {
            self.0.borrow_mut().push((
                entry.stage.to_string(),
                entry.outcome.to_string(),
                entry.spend.cost_usd,
            ));
            Ok(())
        }
    }

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn answered(text: &str) -> Reply {
        Reply {
            text: text.to_string(),
            stop_line: None,
            spend: Spend {
                cost_usd: Some(0.1),
                ..Spend::default()
            },
        }
    }

    async fn run(
        reply: Result<Reply, Halt>,
        dry_run: bool,
    ) -> (Outcome<Reply>, Context<()>, String) {
        let spending = Recorded::default();
        let capture = Rc::new(Capture::default());
        let mut ctx = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            (),
            Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal),
        );
        let mut session = Scripted(reply);
        let result = {
            let mut open = Open {
                ctx: &mut ctx,
                session: &mut session,
            };
            ask_and_record(&mut open, "le prompt", "brief", 2, "32", &spending).await
        };
        let said = capture.0.borrow().join("\n");
        (result, ctx, said)
    }

    #[tokio::test]
    async fn a_successful_reply_is_kept_in_results_under_its_stage_name() {
        let (result, ctx, _) = run(Ok(answered("AGENT_LOOP_OK: fait")), false).await;
        result.expect("une réponse");
        assert_eq!(ctx.results["brief"].text, "AGENT_LOOP_OK: fait");
    }

    #[tokio::test]
    async fn a_stop_marker_halts_and_names_the_stage() {
        let (result, _, _) = run(Ok(answered("AGENT_LOOP_STOP: bloqué")), false).await;
        let err = result.expect_err("doit s'arrêter");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains("(/brief)"));
    }

    #[tokio::test]
    async fn a_missing_ok_marker_is_said_not_treated_as_a_failure() {
        let (result, _, said) = run(Ok(answered("j'ai fini")), false).await;
        assert!(result.is_ok());
        assert!(said.contains("no AGENT_LOOP_OK marker"));
    }

    #[tokio::test]
    async fn a_dead_stage_still_gets_its_line_with_nothing_observed() {
        let (result, _, _) = run(Err(Halt::Quota("épuisé".into())), false).await;
        assert!(matches!(result, Err(Halt::Quota(_))));
    }

    #[tokio::test]
    async fn a_dry_run_records_no_spend_and_asks_for_no_marker() {
        let (result, _, said) = run(Ok(answered("")), true).await;
        assert!(result.is_ok());
        assert!(!said.contains("no AGENT_LOOP_OK marker"));
    }
}
