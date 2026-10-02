//! Ce que l'exécution transporte pendant un run : les réglages, l'état
//! propre au workflow, le journal, ce que chaque étape payante a répondu.

use std::collections::HashMap;

use crate::adapters::agent::Reply;
use crate::traces::Logbook;

/// Ce que l'exécution tient pendant un run, générique sur `S` — l'état
/// propre au workflow qui l'utilise.
///
/// `harness-core` reste ignorant des workflows : il ne nomme jamais un `S`
/// concret, seulement le paramètre. C'est ce qui remplace `RoundCtx(Ctx)` et
/// `RoundState` côté Python, sans downcast — vérifié à la compilation.
pub struct Context<S> {
    /// Les réglages du run.
    pub settings: Settings,
    /// Ce qui appartient au workflow — déclaré par lui, pas par `core`.
    pub state: S,
    /// Le journal de ce run.
    pub traces: Logbook,
    /// Ce que chaque étape payante a répondu, par son nom.
    ///
    /// Ici et pas dans `state` : une réponse alimente l'étape suivante du
    /// même passage, elle ne se relit pas depuis une reprise — elle se
    /// repaie. La boucle de dev n'en lit aucune ; la revue de PR y lit la
    /// passe ligne-à-ligne pour écrire ses notes, le raffinage y lit le
    /// routeur et les sections pour la passe de cohérence.
    pub results: HashMap<String, Reply>,
}

impl<S> Context<S> {
    /// Un contexte neuf, pour cet état et ces réglages.
    #[must_use]
    pub fn new(settings: Settings, state: S, traces: Logbook) -> Self {
        Self {
            settings,
            state,
            traces,
            results: HashMap::new(),
        }
    }
}

/// Les paramètres d'un run.
///
/// Immuables : côté Python, `provisioning.mount()` réécrivait `cfg.workspace`
/// en place, avec le commentaire qu'« un second chemin ferait deux endroits à
/// tenir d'accord ». Ici, monter le workspace rend les `Settings` définitifs
/// *avant* que le `Context` existe — le hack n'a plus d'objet.
#[derive(Debug, Clone)]
pub struct Settings {
    /// N'exécute rien, ne dépense rien — décrit ce qui aurait tourné.
    pub dry_run: bool,
    /// Filtre `--stages` : vide veut dire "tout".
    pub stages: String,
}

impl Settings {
    /// Vrai si `stage` doit tourner selon le filtre `--stages`.
    ///
    /// Vide veut dire "tout" — c'est le défaut, et c'est ce qui permet à
    /// aucun workflow de redire cette règle.
    #[must_use]
    pub fn runs(&self, stage: &str) -> bool {
        self.stages.is_empty() || self.stages.split_whitespace().any(|s| s == stage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(stages: &str) -> Settings {
        Settings {
            dry_run: false,
            stages: stages.to_string(),
        }
    }

    #[test]
    fn empty_filter_runs_everything() {
        assert!(settings("").runs("code"));
    }

    #[test]
    fn filter_runs_only_the_named_stages() {
        let cfg = settings("business-analyst code");
        assert!(cfg.runs("code"));
        assert!(!cfg.runs("create-test"));
    }
}
