//! The journal of a run: verbosity levels and the tag that makes a journal
//! interlaced and attributable.

use std::rc::Rc;

/// What the user sees on standard output. The file keeps everything — this
/// distinction exists only for the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// Everything, including what a session says in detail.
    Verbose,
    /// One line per stage.
    Normal,
    /// Only stops and failures.
    Quiet,
}

/// Where a journal line goes. The only thing `traces/` does not decide itself:
/// write to console, to a file, or nowhere.
pub trait Sink {
    /// Write an already-formatted line.
    fn emit(&self, line: &str);
}

/// A sink that writes nowhere — for tests and the fast path.
struct Null;

impl Sink for Null {
    fn emit(&self, _line: &str) {}
}

/// The journal of a run: a sink, an optional tag, a verbosity level.
///
/// Leaf: knows neither `Halt` nor `Verdict`. The executor that logs a halt
/// passes it an already-formatted string — `traces/` need not know what a halt
/// is to remain a leaf of the package.
#[derive(Clone)]
pub struct Logbook {
    sink: Rc<dyn Sink>,
    verbosity: Verbosity,
    tag: Option<String>,
}

impl Logbook {
    /// A logbook that writes to `sink`, at `verbosity` level.
    #[must_use]
    pub fn new(sink: Rc<dyn Sink>, verbosity: Verbosity) -> Self {
        Self {
            sink,
            verbosity,
            tag: None,
        }
    }

    /// A logbook that writes nowhere.
    #[must_use]
    pub fn null() -> Self {
        Self::new(Rc::new(Null), Verbosity::Quiet)
    }

    /// The same logbook, where each line now bears this tag.
    ///
    /// `log.bind("r3").bind("code")` timestamps lines `[r3:code]` — which makes
    /// an interlaced journal (several stages, one file) attributable.
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

    /// Write a line, with tag at the head if `bind` set one.
    pub fn say(&self, line: &str) {
        match &self.tag {
            Some(tag) => self.sink.emit(&format!("[{tag}] {line}")),
            None => self.sink.emit(line),
        }
    }

    /// A line that a human must see even in `Quiet`.
    ///
    /// Not a failure — a `Halt` travels as a value. What passes here is what
    /// *does not stop* the run and yet would be a bad surprise later: a kept
    /// workspace, local work missing from the clone.
    pub fn warn(&self, line: &str) {
        self.say(&format!("warning: {line}"));
    }

    /// A line of interest only to autopsy — silent unless `Verbose`.
    pub fn debug(&self, line: &str) {
        if self.verbosity == Verbosity::Verbose {
            self.say(line);
        }
    }

    /// The verbosity level of this logbook.
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

    #[test]
    fn a_warning_is_written_even_at_the_quietest_level() {
        // A kept workspace does not stop the run: if `--quiet` swallowed it,
        // nobody would ever learn that work stays on disk.
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Quiet);
        log.warn("workspace kept");
        assert_eq!(capture.0.borrow()[0], "warning: workspace kept");
    }

    #[test]
    fn a_debug_line_only_shows_up_when_asked_for() {
        let quiet = Rc::new(Capture::default());
        Logbook::new(Rc::clone(&quiet) as Rc<dyn Sink>, Verbosity::Normal).debug("detail");
        assert!(quiet.0.borrow().is_empty());

        let loud = Rc::new(Capture::default());
        Logbook::new(Rc::clone(&loud) as Rc<dyn Sink>, Verbosity::Verbose).debug("detail");
        assert_eq!(loud.0.borrow()[0], "detail");
    }
}
