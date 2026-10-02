//! Ce que les étapes d'une revue se transmettent.

use harness_core::domain::Pr;

/// L'état d'une revue : la PR, posée par le précontrôle.
#[derive(Debug, Clone, Default)]
pub struct ReviewState {
    /// La PR revue. `None` jusqu'à ce que `precheck` l'ait lue.
    pub pr: Option<Pr>,
}

impl ReviewState {
    /// La PR, ou l'échec de programmation si on l'atteint avant `precheck`.
    ///
    /// # Panics
    /// Si appelée avant que `precheck` ait tourné — ce qu'aucun chemin de
    /// [`OneShot::execute`](harness_core::execution::OneShot::execute) ne
    /// permet : les stages ne tournent qu'après.
    #[must_use]
    pub const fn pr(&self) -> &Pr {
        self.pr
            .as_ref()
            .expect("precheck doit poser la PR avant que les stages tournent")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_has_no_pr_yet() {
        assert!(ReviewState::default().pr.is_none());
    }
}
