//! Ce qu'un état de round doit savoir pour qu'une étape ne soit jamais
//! repayée deux fois.

/// Porté par l'état d'un workflow : au plus une fois par étape, tous runs confondus.
///
/// Remplace le `done: list[str]` passé à chaque appel côté Python, et le cas
/// particulier `mark=False` du stage d'archivage.
pub trait Resumable {
    /// Les étapes déjà faites pour la task en cours.
    fn done(&self) -> &[String];

    /// Marque une étape faite. Idempotent : la marquer deux fois ne duplique
    /// rien dans `done()`.
    fn mark(&mut self, stage: &str);

    /// Vrai si cette étape a déjà tourné.
    fn is_done(&self, stage: &str) -> bool {
        self.done().iter().any(|s| s == stage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct State {
        done: Vec<String>,
    }

    impl Resumable for State {
        fn done(&self) -> &[String] {
            &self.done
        }

        fn mark(&mut self, stage: &str) {
            if !self.is_done(stage) {
                self.done.push(stage.to_string());
            }
        }
    }

    #[test]
    fn marking_twice_does_not_duplicate() {
        let mut state = State::default();
        state.mark("code");
        state.mark("code");
        assert_eq!(state.done(), &["code".to_string()]);
    }

    #[test]
    fn is_done_reflects_what_was_marked() {
        let mut state = State::default();
        assert!(!state.is_done("code"));
        state.mark("code");
        assert!(state.is_done("code"));
    }
}
