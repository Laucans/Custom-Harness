//! Le texte que la revue publie sur la PR — le seul métier de la revue.
//!
//! Tout est ici parce que tout est publié : le marqueur qui dit qu'une PR en
//! porte déjà une, et le pied de page. Rien n'y lit un fichier ni n'appelle un
//! binaire, donc tout se relit et se change sans faire tourner quoi que ce
//! soit.
//!
//! Ce texte **reste en français** : il est publié sur une PR GitHub pour un
//! humain francophone.

use harness_core::domain::prompts::splice;

/// Ce qui dit qu'une PR porte déjà une revue de ce harness.
pub const MARKER: &str = "<!-- agent-review -->";

const FOOTER: &str = "_Revue automatique (`harness`, workflow `pr_review`) — {passes}{cost}. \
Indicative : elle ne bloque rien et le lot a pu être mergé entre-temps. Les \
findings ligne à ligne sont dans l'onglet **Files changed**._";

/// Le pied de page, les passes et le coût substitués.
#[must_use]
pub fn footer(passes: &str, cost: &str) -> String {
    splice(FOOTER, &[("passes", passes), ("cost", cost)])
}

/// Le commentaire complet, tel qu'il est posté.
#[must_use]
pub fn comment(stamp: &str, body: &str, footer: &str) -> String {
    format!("{MARKER}\n## 🤖 Notes de revue — {stamp}\n\n{body}\n\n---\n{footer}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_footer_substitutes_both_fields() {
        let said = footer(
            "findings `sonnet`/niveau `medium`, notes `sonnet`",
            ", coût $0.42",
        );
        assert!(said.contains("findings `sonnet`"));
        assert!(said.contains("coût $0.42"));
    }

    #[test]
    fn the_comment_carries_the_marker_first_so_a_later_run_can_find_it() {
        let said = comment("2026-10-02 16:00", "le corps", "le pied de page");
        assert!(said.starts_with(MARKER));
        assert!(said.contains("le corps"));
        assert!(said.contains("le pied de page"));
    }
}
