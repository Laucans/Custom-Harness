//! Les arrêts, comme valeurs : `Halt` remplace l'exception.

use thiserror::Error;

/// Le niveau auquel un arrêt mérite d'être journalisé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Un résultat correct, pas un problème.
    Info,
    /// La fenêtre d'abonnement est épuisée — ni réparé ni abandonné.
    Warn,
    /// Un stage n'a rien rendu d'utilisable.
    Error,
}

/// Les quatre façons dont un run s'arrête sans lever.
///
/// Codes de sortie, préfixes et niveaux sont ceux que lisait déjà
/// l'ordonnanceur externe du pipeline Python — inchangés par la migration :
/// un ordonnanceur extérieur les lit, et les déplacer serait un changement
/// de contrat déguisé en refactoring.
#[derive(Debug, Clone, Error)]
pub enum Halt {
    /// L'arrêt volontaire : la boucle refuse de deviner.
    #[error("{0}")]
    Halted(String),
    /// Un magasin (GitHub, le point de reprise) n'a pas répondu.
    ///
    /// Séparé de [`Halt::Halted`] : lire "illisible" comme "rien à faire"
    /// ferait payer un `/planner` pour un jeton expiré.
    #[error("{0}")]
    Unreadable(String),
    /// Un stage n'a rien rendu d'utilisable. Pas un résultat correct.
    #[error("{0}")]
    Failed(String),
    /// La fenêtre d'abonnement est épuisée — revenir plus tard, inchangé.
    #[error("{0}")]
    Quota(String),
}

impl Halt {
    /// Le code que lit un ordonnanceur extérieur.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Halted(_) | Self::Unreadable(_) => 1,
            Self::Failed(_) => 2,
            Self::Quota(_) => 3,
        }
    }

    /// Comment la ligne s'annonce dans le journal (`STOP`, `FAILED`, `QUOTA`).
    #[must_use]
    pub const fn prefix(&self) -> &'static str {
        match self {
            Self::Halted(_) | Self::Unreadable(_) => "STOP",
            Self::Failed(_) => "FAILED",
            Self::Quota(_) => "QUOTA",
        }
    }

    /// Le niveau auquel journaliser cet arrêt.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        match self {
            Self::Halted(_) | Self::Unreadable(_) => Severity::Info,
            Self::Failed(_) => Severity::Error,
            Self::Quota(_) => Severity::Warn,
        }
    }

    /// La raison portée, quelle que soit la variante.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Halted(r) | Self::Unreadable(r) | Self::Failed(r) | Self::Quota(r) => r,
        }
    }

    /// Le même arrêt, avec la raison englobante ajoutée devant.
    ///
    /// La raison d'origine est gardée derrière : ce qui a cassé en bas et ce
    /// que ça empêchait en haut sont deux moitiés de la même phrase, et une
    /// autopsie a besoin des deux.
    #[must_use]
    pub fn but(self, context: &str) -> Self {
        let said = format!("{context} ({})", self.reason());
        match self {
            Self::Halted(_) => Self::Halted(said),
            Self::Unreadable(_) => Self::Unreadable(said),
            Self::Failed(_) => Self::Failed(said),
            Self::Quota(_) => Self::Quota(said),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_the_frozen_contract() {
        assert_eq!(Halt::Halted(String::new()).exit_code(), 1);
        assert_eq!(Halt::Unreadable(String::new()).exit_code(), 1);
        assert_eq!(Halt::Failed(String::new()).exit_code(), 2);
        assert_eq!(Halt::Quota(String::new()).exit_code(), 3);
    }

    #[test]
    fn but_keeps_the_original_reason_behind_the_new_one() {
        let halt = Halt::Failed("token expired".into()).but("refresh failed");
        assert_eq!(halt.reason(), "refresh failed (token expired)");
    }

    #[test]
    fn halted_and_unreadable_are_both_info_but_stay_distinct_variants() {
        assert_eq!(Halt::Halted(String::new()).severity(), Severity::Info);
        assert_eq!(Halt::Unreadable(String::new()).severity(), Severity::Info);
        assert!(matches!(Halt::Halted(String::new()), Halt::Halted(_)));
        assert!(matches!(
            Halt::Unreadable(String::new()),
            Halt::Unreadable(_)
        ));
    }
}
