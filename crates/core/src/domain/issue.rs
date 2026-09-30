//! Ce qu'est une issue, et rien de ce qu'un workflow en fait.
//!
//! Le suivi du travail vit dans les issues GitHub. Ce module en porte la
//! **forme** — un numéro, un titre, un état, des étiquettes, un corps, des
//! bloqueurs — et les seules questions qu'on peut lui poser sans savoir à quoi
//! elle sert.
//!
//! **Le vocabulaire, pas la définition.** Ce qui fait d'une issue une « task
//! prête à tourner » — quelles étiquettes comptent, laquelle choisir ensuite —
//! est la définition d'un workflow et vit chez lui. C'est cette séparation qui
//! autorise `adapters::shell::github` à rendre des `Issue` : un adaptateur
//! désérialise, il ne décide pas de ce qu'une étiquette *signifie*.

/// Une issue GitHub, vue par le harness.
///
/// Le même type sert pour un item de roadmap, un milestone, une task, une
/// action humaine et une PR : ce qui les distingue est une étiquette, pas une
/// classe. C'est ce qui permet à `blocked_by` de porter des issues de
/// n'importe quelle sorte — une dépendance ne demande pas ce qu'elle bloque.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Issue {
    /// Son numéro.
    pub number: u64,
    /// Son titre.
    pub title: String,
    /// `open` ou `closed`, tel que l'API le dit.
    pub state: String,
    /// Ses étiquettes, par leur nom.
    pub labels: Vec<String>,
    /// Son corps. Pour une task, le corps **est** le SPEC.
    pub body: String,
    /// Ses bloqueurs, **avec leur état** : savoir qu'une issue est bloquée ne
    /// suffit pas, il faut savoir si le bloqueur est encore ouvert.
    pub blocked_by: Vec<Self>,
}

impl Issue {
    /// L'identité de l'issue pour l'état de reprise : son numéro.
    ///
    /// Un numéro ne change pas quand on réécrit le titre — ce qui n'était pas
    /// vrai du temps où une task se désignait par `<num>|<titre>`.
    #[must_use]
    pub fn key(&self) -> String {
        self.number.to_string()
    }

    /// `#12` — comme un message la nomme.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("#{}", self.number)
    }

    /// Vrai tant que GitHub ne l'a pas fermée.
    ///
    /// Tout ce qui n'est pas `closed` compte comme ouvert : un état que cette
    /// version ne connaît pas ne doit pas faire disparaître du travail.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.state != "closed"
    }

    /// Vrai si GitHub l'a fermée.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.state == "closed"
    }

    /// Vrai si elle porte cette étiquette.
    #[must_use]
    pub fn has(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(state: &str, labels: &[&str]) -> Issue {
        Issue {
            number: 12,
            state: state.to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    #[test]
    fn an_unknown_state_counts_as_open_rather_than_losing_the_work() {
        let odd = issue("something_new", &[]);
        assert!(odd.is_open());
        assert!(!odd.is_closed());
    }

    #[test]
    fn closed_is_the_only_state_that_closes() {
        assert!(issue("closed", &[]).is_closed());
        assert!(!issue("closed", &[]).is_open());
    }

    #[test]
    fn labels_are_matched_exactly_not_by_prefix() {
        let task = issue("open", &["pipeline:agent"]);
        assert!(task.has("pipeline:agent"));
        assert!(!task.has("pipeline"));
    }

    #[test]
    fn the_key_is_the_number_so_a_retitled_task_keeps_its_identity() {
        let mut task = issue("open", &[]);
        let before = task.key();
        task.title = "un titre tout neuf".to_string();
        assert_eq!(task.key(), before);
        assert_eq!(task.reference(), "#12");
    }
}
