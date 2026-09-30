//! Le seul endroit qui sait qu'une session d'agent doit être ouverte.
//!
//! Miroir de `adapters/agent/base.py` côté Python (`AgentRunner`, une ABC,
//! une seule implémentation qui importe le SDK) : une interface décidée
//! maintenant, un porteur concret décidé derrière elle, plus tard. Ce module
//! ne choisit pas de porteur — tmux, un processus `claude -p
//! --input-format stream-json`, ou autre chose : c'est une décision
//! d'implémentation, prise une fois le porteur tranché (`docs/MIGRATION.md`,
//! « Ce qui reste à trancher »).

pub mod claude_cli;

use async_trait::async_trait;

use crate::domain::Outcome;

/// Ce qu'une session rend après un tour.
///
/// Honnête sur ce qu'un porteur peut réellement observer : `cost` est
/// optionnel parce qu'un pane de terminal ne rend pas de compte d'usage,
/// contrairement à un flux JSON structuré. Le trait ne présuppose pas la
/// capacité du porteur le plus généreux.
#[derive(Debug, Clone)]
pub struct Reply {
    /// Le texte que la session a rendu pour ce tour.
    pub text: String,
    /// Le marqueur `AGENT_LOOP_STOP`, si la session s'est arrêtée d'elle-même.
    pub stop_line: Option<String>,
    /// Le coût de ce tour, en dollars, quand le porteur peut le rendre.
    pub cost: Option<f64>,
}

/// Une session ouverte contre un agent : plusieurs tours, un seul processus.
///
/// Ne quitte jamais sa `Stage` — rien en dehors d'une stage ne détient de
/// `Session`. Ce n'est pas une convention : aucun type public n'expose un
/// moyen d'en obtenir une autrement qu'à l'intérieur d'un `perform` de
/// `Stage`.
#[async_trait(?Send)]
pub trait Session {
    /// Envoie un message dans la session ouverte, attend le tour complet.
    async fn ask(&mut self, prompt: &str) -> Outcome<Reply>;
}

/// Ce qu'il faut pour ouvrir une session : le modèle, l'effort.
///
/// Pure donnée — un porteur tmux ou stream-json en fait ce qu'il veut, mais
/// aucun des deux ne change ce qu'un appelant a le droit de demander.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// Le modèle demandé (`"opus"`, `"sonnet"`, …).
    pub model: String,
    /// Le niveau d'effort (`"low"` … `"max"`).
    pub effort: String,
}

/// Construit une [`Session`].
///
/// La seule chose que `harness-core` sait d'un porteur concret : qu'il en
/// existe un, injecté par la stage qui en a besoin. Miroir de `hub.py` :
/// « seul endroit qui construit un client — une seule couture de test pour
/// tous les workflows. »
#[async_trait(?Send)]
pub trait SessionFactory {
    /// Ouvre une session neuve pour ce `spec`.
    async fn open(&self, spec: &SessionSpec) -> Outcome<Box<dyn Session>>;
}
