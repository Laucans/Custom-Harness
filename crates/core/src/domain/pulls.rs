//! Ce qu'on lit d'une PR. Métier pur : ni I/O, ni sous-processus.

/// Une PR, telle qu'une revue la lit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pr {
    /// Son numéro, en texte — il n'entre que dans de la prose et des chemins.
    pub num: String,
    /// La branche visée.
    pub base: String,
    /// La branche qui porte le changement.
    pub head: String,
    /// Son titre.
    pub title: String,
    /// Son URL.
    pub url: String,
    /// `OPEN`, `CLOSED`, `MERGED` — tel que l'API le rend, non interprété.
    pub state: String,
    /// Un brouillon n'a rien à faire relire.
    pub draft: bool,
}

impl Pr {
    /// `#12` — comme un message le nomme.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("#{}", self.num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_is_the_number_prefixed() {
        let pr = Pr {
            num: "32".to_string(),
            ..Pr::default()
        };
        assert_eq!(pr.reference(), "#32");
    }
}
