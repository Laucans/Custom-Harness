//! Le contrat verbal : ce qu'une session dit pour qu'on sache où elle en est.
//!
//! Une session sans `AGENT_LOOP_OK` retombe sur les vérifications
//! structurelles — les gates. Le marqueur n'est pas la preuve qu'un travail
//! est fait, c'est la version courte ; c'est une PR mergée qui prouve.

/// La session dit ce qu'elle a fait.
pub const OK: &str = "AGENT_LOOP_OK";

/// La session s'arrête d'elle-même. Un résultat correct, pas une panne.
pub const STOP: &str = "AGENT_LOOP_STOP";

/// La ligne qui porte [`STOP`], si le texte en contient une.
///
/// Rendue entière plutôt qu'en booléen : la raison que la session donne est
/// sur cette ligne, et c'est elle qu'un humain lira dans le journal.
#[must_use]
pub fn stop_line(text: &str) -> Option<String> {
    text.lines()
        .find(|line| line.contains(STOP))
        .map(|line| line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_without_the_marker_has_no_stop_line() {
        assert!(stop_line("j'ai fini, tout va bien").is_none());
    }

    #[test]
    fn the_whole_line_comes_back_not_just_the_marker() {
        let text = "voici ce que j'ai fait\nAGENT_LOOP_STOP: le SPEC est vide\nfin";
        assert_eq!(
            stop_line(text).as_deref(),
            Some("AGENT_LOOP_STOP: le SPEC est vide")
        );
    }

    #[test]
    fn the_line_is_trimmed_so_indentation_does_not_leak_into_the_journal() {
        assert_eq!(
            stop_line("   AGENT_LOOP_STOP: rien à faire   ").as_deref(),
            Some("AGENT_LOOP_STOP: rien à faire")
        );
    }

    #[test]
    fn ok_and_stop_are_distinct_markers() {
        assert!(stop_line("AGENT_LOOP_OK: livré").is_none());
    }
}
