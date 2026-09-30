//! L'état d'un round de la boucle : le `S` de `Context<S>`.
//!
//! Il remplace les trois objets qui se chevauchaient côté Python — `Ctx`,
//! `RoundCtx(Ctx)` et `RoundState`. Un seul type, et les deux bornes que le
//! core exige de lui : [`Resumable`] pour l'idempotence, [`Scoped`] pour que
//! nulle session ne démarre sans savoir sur quoi elle travaille.

use harness_core::domain::{Named, Resumable, Scope, Scoped};

/// Ce que le round sait, et qui n'appartient qu'à lui.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loop {
    /// Le milestone en cours.
    pub milestone: Named,
    /// La task choisie. Vide tant que `pick-task` n'a pas tourné.
    pub task: Named,
    /// La clé de reprise de la task — son numéro.
    pub task_key: String,
    /// `auto` ou `human`, pour le journal.
    pub kind: String,
    /// Vrai quand il n'y a aucune task : le round bifurque vers `/planner`.
    pub rollover: bool,
    /// Le SPEC est déjà dans le corps de l'issue.
    pub spec_written: bool,
    /// Les stages déjà faites pour cette task, tous runs confondus.
    pub stages_done: Vec<String>,
}

impl Resumable for Loop {
    fn done(&self) -> &[String] {
        &self.stages_done
    }

    fn mark(&mut self, stage: &str) {
        if !self.is_done(stage) {
            self.stages_done.push(stage.to_string());
        }
    }
}

impl Scoped for Loop {
    fn scope(&self) -> Scope {
        Scope {
            milestone: self.milestone.clone(),
            task: self.task.clone(),
        }
    }
}

impl Loop {
    /// Vrai si une task a été choisie.
    #[must_use]
    pub const fn has_task(&self) -> bool {
        !self.task_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Loop {
        Loop {
            milestone: Named {
                number: "12".to_string(),
                title: "Le chat".to_string(),
                body: "le milestone".to_string(),
            },
            task: Named {
                number: "34".to_string(),
                title: "La grille".to_string(),
                body: "le SPEC".to_string(),
            },
            task_key: "34".to_string(),
            kind: "auto".to_string(),
            ..Loop::default()
        }
    }

    #[test]
    fn a_fresh_state_has_no_task_and_is_not_a_rollover() {
        let fresh = Loop::default();
        assert!(!fresh.has_task());
        assert!(!fresh.rollover);
        assert!(fresh.done().is_empty());
    }

    #[test]
    fn marking_a_stage_twice_does_not_duplicate_it() {
        // L'idempotence est au core, via cette borne : sans elle il faudrait
        // repasser la liste à chaque appel, comme le Python le faisait.
        let mut round = state();
        round.mark("code");
        round.mark("code");
        assert_eq!(round.done(), &["code".to_string()]);
        assert!(round.is_done("code"));
        assert!(!round.is_done("create-test"));
    }

    #[test]
    fn the_scope_carries_both_issues_so_no_session_starts_blind() {
        let scope = state().scope();
        assert_eq!(scope.milestone.number, "12");
        assert_eq!(scope.task.title, "La grille");
        assert_eq!(scope.task.body, "le SPEC");
    }
}
