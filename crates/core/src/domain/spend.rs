//! Ce qu'un tour a coûté. Donnée pure : personne ici ne sait l'écrire.
//!
//! Chaque champ est un `Option`, et pour la même raison que
//! `adapters::agent::Reply::cost` : un porteur de session ne rend pas
//! forcément tout. Un pane de terminal ne rend ni jetons ni coût ; un flux
//! JSON rend les deux. `None` se lit « non observé », jamais « zéro » — la
//! différence est ce qui empêche un registre de compter une session gratuite
//! là où il n'a rien su mesurer.

/// Les jetons d'un tour, par nature.
///
/// Séparés du coût : le coût est une estimation dérivée d'une table de prix,
/// les jetons sont ce que l'API a réellement rapporté.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Jetons d'entrée, cache exclu.
    pub input: Option<u64>,
    /// Jetons de sortie.
    pub output: Option<u64>,
    /// Jetons lus dans le cache, facturés à taux réduit.
    pub cache_read: Option<u64>,
    /// Jetons écrits dans le cache, facturés à taux majoré.
    pub cache_write: Option<u64>,
}

/// Ce qu'un tour a coûté, et ce qu'il a consommé.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spend {
    /// Coût en dollars.
    ///
    /// **Estimation côté client**, calculée d'une table de prix embarquée dans
    /// Claude Code — pas une donnée de facturation. Bonne pour un budget,
    /// jamais pour facturer.
    ///
    /// Attention à la sémantique du cumul : sur une session reprise, Claude
    /// Code rend le total de **toute** la conversation depuis la v2.1.277, et
    /// seulement celui de l'appel avant. Voir
    /// `adapters::agent::claude_cli`.
    pub cost_usd: Option<f64>,
    /// Combien d'aller-retours avec le modèle ce tour a demandés.
    pub turns: Option<u32>,
    /// Durée du tour, bout en bout.
    pub duration_ms: Option<u64>,
    /// Les jetons consommés.
    pub tokens: Tokens,
    /// L'identifiant de session que le porteur a rapporté.
    pub session: Option<String>,
}

impl Spend {
    /// Vrai si rien n'a été observé — aucun champ rempli.
    ///
    /// Ce qu'un registre lit pour écrire « non mesuré » plutôt qu'une colonne
    /// de zéros, qui se relirait comme une session gratuite.
    #[must_use]
    pub const fn is_blind(&self) -> bool {
        self.cost_usd.is_none()
            && self.turns.is_none()
            && self.duration_ms.is_none()
            && self.tokens.input.is_none()
            && self.tokens.output.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_spend_is_blind() {
        assert!(Spend::default().is_blind());
    }

    #[test]
    fn one_observed_field_is_enough_to_stop_being_blind() {
        let spend = Spend {
            cost_usd: Some(0.0),
            ..Spend::default()
        };
        // Un coût de zéro **observé** n'est pas la même chose que rien
        // d'observé : le premier est une mesure, le second une ignorance.
        assert!(!spend.is_blind());
    }

    #[test]
    fn tokens_default_to_unobserved_not_to_zero() {
        let tokens = Tokens::default();
        assert_eq!(tokens.input, None);
        assert_eq!(tokens.cache_read, None);
    }
}
