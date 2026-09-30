//! Le journal d'un run : niveaux, et l'étiquette qui rend un journal
//! entrelacé attribuable.

use std::rc::Rc;

/// Ce que l'utilisateur voit sur la sortie standard. Le fichier, lui, garde
/// toujours tout — cette distinction n'existe que pour la console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// Tout, y compris ce qu'une session dit en détail.
    Verbose,
    /// Une ligne par étape.
    Normal,
    /// Seuls les arrêts et les échecs.
    Quiet,
}

/// Où une ligne de journal part. Le seul point que `traces/` ne décide pas
/// lui-même : écrire sur la console, dans un fichier, ou nulle part.
pub trait Sink {
    /// Écrit une ligne déjà formatée.
    fn emit(&self, line: &str);
}

/// Un puits qui n'écrit nulle part — pour les tests et le chemin rapide.
struct Null;

impl Sink for Null {
    fn emit(&self, _line: &str) {}
}

/// Le journal d'un run : un puits, une étiquette optionnelle, un niveau.
///
/// Feuille : ne connaît ni `Halt` ni `Verdict`. L'exécuteur qui journalise un
/// arrêt lui passe une chaîne déjà formatée — `traces/` n'a pas à savoir ce
/// qu'un arrêt est pour rester une feuille du paquet.
#[derive(Clone)]
pub struct Logbook {
    sink: Rc<dyn Sink>,
    verbosity: Verbosity,
    tag: Option<String>,
}

impl Logbook {
    /// Un journal qui écrit vers `sink`, au niveau `verbosity`.
    #[must_use]
    pub fn new(sink: Rc<dyn Sink>, verbosity: Verbosity) -> Self {
        Self {
            sink,
            verbosity,
            tag: None,
        }
    }

    /// Un journal qui n'écrit nulle part.
    #[must_use]
    pub fn null() -> Self {
        Self::new(Rc::new(Null), Verbosity::Quiet)
    }

    /// Le même journal, dont chaque ligne porte désormais cette étiquette.
    ///
    /// `log.bind("r3").bind("code")` estampille les lignes `[r3:code]` — ce
    /// qui rend un journal entrelacé (plusieurs stages, un seul fichier)
    /// attribuable.
    #[must_use]
    pub fn bind(&self, tag: &str) -> Self {
        let tag = self
            .tag
            .as_ref()
            .map_or_else(|| tag.to_string(), |existing| format!("{existing}:{tag}"));
        Self {
            sink: Rc::clone(&self.sink),
            verbosity: self.verbosity,
            tag: Some(tag),
        }
    }

    /// Écrit une ligne, étiquette en tête si `bind` en a posé une.
    pub fn say(&self, line: &str) {
        match &self.tag {
            Some(tag) => self.sink.emit(&format!("[{tag}] {line}")),
            None => self.sink.emit(line),
        }
    }

    /// Le niveau de ce journal.
    #[must_use]
    pub const fn verbosity(&self) -> Verbosity {
        self.verbosity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    #[test]
    fn bind_prefixes_every_line_that_follows() {
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal).bind("r3");
        log.say("starting");
        assert_eq!(capture.0.borrow()[0], "[r3] starting");
    }

    #[test]
    fn binding_twice_composes_the_tags() {
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal)
            .bind("r3")
            .bind("code");
        log.say("hi");
        assert_eq!(capture.0.borrow()[0], "[r3:code] hi");
    }

    #[test]
    fn an_unbound_logbook_writes_the_line_as_is() {
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        log.say("no tag here");
        assert_eq!(capture.0.borrow()[0], "no tag here");
    }

    #[test]
    fn null_never_panics_and_records_nothing() {
        Logbook::null().say("nowhere");
    }
}
