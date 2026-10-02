//! Le porteur d'un `--dry-run` : il rend le prompt, il n'ouvre rien.
//!
//! Un dry-run **est un choix de câblage**, pas une branche du framework. La
//! stage ouvre sa session sans savoir ce qu'il y a derrière ; le lanceur
//! décide que, cette fois, derrière il y a ceci. Côté Python, chaque fonction
//! qui exécutait portait son `if cfg.dry_run`, et le chemin à blanc était donc
//! un second parcours du code — celui qu'aucun test d'intégration ne couvre.
//!
//! Ce qu'un dry-run montre : le prompt exact qui serait envoyé, stage par
//! stage, portée comprise. C'est la question à laquelle il sert à répondre.

use async_trait::async_trait;

use crate::adapters::agent::{Reply, Session, SessionFactory, SessionSpec};
use crate::domain::{Outcome, Spend};
use crate::traces::Logbook;

/// Ouvre des sessions qui n'en sont pas.
pub struct Rehearsal {
    log: Logbook,
}

impl Rehearsal {
    /// Une répétition qui écrit ce qu'elle aurait envoyé dans ce journal.
    #[must_use]
    pub const fn new(log: Logbook) -> Self {
        Self { log }
    }
}

#[async_trait(?Send)]
impl SessionFactory for Rehearsal {
    async fn open(&self, spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
        self.log.say(&format!(
            "dry-run — aucune session ouverte (aurait été {}, effort {})",
            spec.model, spec.effort
        ));
        Ok(Box::new(Transcript {
            log: self.log.clone(),
        }))
    }
}

/// Une session qui recopie ce qu'on lui dit, et ne répond rien.
struct Transcript {
    log: Logbook,
}

#[async_trait(?Send)]
impl Session for Transcript {
    async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
        self.log
            .say(&format!("dry-run — le prompt qui partirait :\n{prompt}"));
        // Texte vide, et donc aucun marqueur : un appelant qui réclamerait
        // `AGENT_LOOP_OK` ici lirait l'absence de réponse comme un échec, alors
        // que personne n'a été interrogé. C'est à lui de ne pas le réclamer en
        // dry-run, et `Spend::default()` dit « rien d'observé », pas « gratuit ».
        Ok(Reply {
            text: String::new(),
            stop_line: None,
            spend: Spend::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::{Sink, Verbosity};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn spec() -> SessionSpec {
        SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        }
    }

    #[tokio::test]
    async fn a_rehearsal_writes_the_prompt_it_would_have_sent() {
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        let mut session = Rehearsal::new(log).open(&spec()).await.expect("ouverte");
        session
            .ask("/code\nles consignes")
            .await
            .expect("une réponse");
        let said = capture.0.borrow().join("\n");
        assert!(said.contains("aucune session ouverte"));
        assert!(
            said.contains("les consignes"),
            "le prompt exact, pas un résumé"
        );
    }

    #[tokio::test]
    async fn a_rehearsal_observes_nothing_rather_than_reporting_zero() {
        // Une ligne de registre à zéro se relirait comme une session gratuite.
        let mut session = Rehearsal::new(Logbook::null())
            .open(&spec())
            .await
            .expect("ouverte");
        let reply = session.ask("peu importe").await.expect("une réponse");
        assert!(reply.spend.is_blind());
        assert!(reply.stop_line.is_none());
    }
}
