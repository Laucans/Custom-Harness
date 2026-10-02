//! Ce qu'un round reçoit du lanceur : ses ports, et ce que ce run-ci change.
//!
//! Séparé des `Settings` du core à dessein. `Settings` porte ce que **tout**
//! workflow a — `--dry-run`, `--stages` — et le core le lit. Ce qui est ici
//! n'appartient qu'à la boucle : sa branche d'intégration, le forçage de
//! modèle, et les trois ports dont ses actions ont besoin.
//!
//! Aucun `Rc` n'est cloné à chaque stage : la table en prend un par action qui
//! en a besoin, une fois, au montage.

use std::rc::Rc;

use harness_core::adapters::agent::{SessionFactory, SessionSpec};
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::spending::Spending;

/// Ce que ce workflow cite comme ayant injecté le prompt.
///
/// Ici et pas dans `harness-core::domain::prompts` : c'est le point d'entrée de
/// **ce** workflow, et une session qui lit ce nom doit pouvoir aller voir le
/// fichier. Le core, lui, ne nomme aucune instance — son défaut reste
/// [`prompts::INJECTOR`].
pub const INJECTOR: &str = "harness-launcher (dev_loop)";

/// Les ports et les réglages d'un run de la boucle.
pub struct Wiring {
    /// Le tableau d'issues, et les étiquettes qu'on y pose.
    pub gh: Rc<dyn GitHub>,
    /// Ce qui ouvre une session payante — ou la répète à blanc.
    pub sessions: Rc<dyn SessionFactory>,
    /// Où la dépense d'un stage est consignée.
    pub spending: Rc<dyn Spending>,
    /// La branche sur laquelle la boucle travaille et merge.
    pub integration_branch: String,
    /// Forçage de modèle pour tout le run. Vide : chaque stage garde le sien.
    pub model: String,
    /// Forçage d'effort pour tout le run. Vide : idem.
    pub effort: String,
    /// Rejoue un stage que la reprise ferait sauter.
    pub restart: bool,
}

impl Wiring {
    /// Le modèle et l'effort de ce stage, forçage du run appliqué.
    ///
    /// `MODEL`/`EFFORT` sont le forçage brutal : une valeur pour tout le run.
    /// Appliqué ici et à un seul endroit — côté Python, `resolve` reconstruisait
    /// un `StageSpec` entier, et un champ ajouté à la table se perdait dès
    /// qu'un run donnait `--model`.
    #[must_use]
    pub fn spec(&self, model: &str, effort: &str) -> SessionSpec {
        SessionSpec {
            model: pick(&self.model, model),
            effort: pick(&self.effort, effort),
        }
    }

    /// Le nom cité comme injecteur dans les prompts de ce workflow.
    #[must_use]
    pub const fn injector(&self) -> &'static str {
        INJECTOR
    }
}

fn pick(forced: &str, default: &str) -> String {
    if forced.is_empty() {
        default.to_string()
    } else {
        forced.to_string()
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! Un câblage qui ne mène nulle part, pour monter une table sans réseau.
    //!
    //! Les deux ports y **refusent** plutôt que de ne rien faire : construire
    //! une table ne doit ouvrir aucune session et ne rien consigner, et un port
    //! muet laisserait passer le contraire sans qu'un test le voie.

    use std::rc::Rc;

    use async_trait::async_trait;
    use harness_core::adapters::agent::{Session, SessionFactory, SessionSpec};
    use harness_core::adapters::store::spending::{Entry, Spending};
    use harness_core::domain::{Halt, Outcome};

    use super::Wiring;
    use crate::dev_loop::board::fake::FakeGitHub;

    /// Une fabrique qui refuse d'ouvrir.
    pub struct NoSessions;

    #[async_trait(?Send)]
    impl SessionFactory for NoSessions {
        async fn open(&self, _spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
            Err(Halt::Failed(
                "aucune session ne doit s'ouvrir dans ce test".to_string(),
            ))
        }
    }

    /// Un registre qui refuse d'écrire.
    pub struct Nowhere;

    impl Spending for Nowhere {
        fn record(&self, _entry: &Entry<'_>) -> Outcome<()> {
            Err(Halt::Failed(
                "aucune dépense ne doit être consignée dans ce test".to_string(),
            ))
        }
    }

    /// Un câblage de test, GitHub vide.
    pub fn wiring() -> Wiring {
        with(Rc::new(FakeGitHub::default()))
    }

    /// Un câblage de test contre ce GitHub-là.
    pub fn with(gh: Rc<FakeGitHub>) -> Wiring {
        Wiring {
            gh,
            sessions: Rc::new(NoSessions),
            spending: Rc::new(Nowhere),
            integration_branch: "main_agent".to_string(),
            model: String::new(),
            effort: String::new(),
            restart: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::prompts;

    #[test]
    fn the_injector_this_workflow_names_is_not_the_cores_placeholder() {
        // Le core reste générique ; une session qui lit « injected by … » doit
        // pouvoir ouvrir le fichier qui l'a injecté.
        assert_ne!(INJECTOR, prompts::INJECTOR);
    }

    #[test]
    fn without_a_forced_model_each_stage_keeps_its_own() {
        let spec = fake::wiring().spec("sonnet", "high");
        assert_eq!(spec.model, "sonnet");
        assert_eq!(spec.effort, "high");
    }

    #[test]
    fn a_forced_model_overrides_every_stage_of_the_run() {
        let mut forced = fake::wiring();
        forced.model = "opus".to_string();
        let spec = forced.spec("sonnet", "high");
        assert_eq!(spec.model, "opus");
        // Forcer l'un ne force pas l'autre : ce sont deux variables.
        assert_eq!(spec.effort, "high");
    }
}
