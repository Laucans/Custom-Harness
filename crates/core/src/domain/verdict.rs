//! Ce qu'un exécutable rend : pas sa donnée, le contrôle de ce qui suit.

use crate::domain::halt::Halt;

/// Le résultat d'un `execute()` qui n'a pas arrêté la séquence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Le travail a été fait, ou la séquence peut enchaîner.
    Continue,
    /// Rien à faire ici ; la raison est journalisée par l'appelant.
    Skip(String),
    /// Il n'y a plus rien à faire **du tout** — un succès, pas un arrêt.
    ///
    /// Distinct de [`Halt`] : "plus de tour à jouer" et "quelque chose a
    /// cassé" ne doivent jamais partager un code de sortie. Seul un `Round`
    /// l'émet ; la répétition s'arrête dessus et rend un succès.
    NothingLeft(String),
}

/// Ce que rend tout exécutable : le contrôle, jamais la charge utile.
///
/// La donnée produite va dans le `Context` ; `Outcome` ne porte que de quoi
/// décider si la séquence continue, saute, s'arrête proprement (`NothingLeft`
/// via `Verdict`), ou a échoué (`Err`).
pub type Outcome<T> = Result<T, Halt>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_and_nothing_left_are_not_the_same_verdict() {
        assert_eq!(Verdict::Skip("a".into()), Verdict::Skip("a".into()));
        assert_ne!(Verdict::Skip("a".into()), Verdict::NothingLeft("a".into()));
    }
}
