//! The journal of a run: verbosity levels and the tag that makes a journal
//! interlaced and attributable.

use std::rc::Rc;

use super::Event;

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

    /// Keep a line without showing it.
    ///
    /// What [`Logbook::debug`] writes when the console is not `Verbose`: a
    /// sink that has somewhere durable to write keeps it there, a sink that is
    /// only a console drops it. Defaults to [`Sink::emit`] — a sink that makes
    /// no such distinction loses nothing by showing it.
    fn keep(&self, line: &str) {
        self.emit(line);
    }

    /// Keep an event as data, besides its line.
    ///
    /// Defaults to nothing: a console, a test sink, has nowhere to keep it,
    /// and the line it already received tells the human.
    fn record(&self, _event: &Event) {}
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

    /// Tell an event: write its line (a warning when it warns), and keep it
    /// as data. One call, so the line and the record cannot disagree.
    pub fn event(&self, event: &Event) {
        if let Some(line) = event.line() {
            if event.warns() {
                self.warn(&line);
            } else {
                self.say(&line);
            }
        }
        self.sink.record(event);
    }

    /// Keep an event as data only — one whose line is a ledger row.
    pub fn record(&self, event: &Event) {
        self.sink.record(event);
    }

    /// Write a line, with tag at the head if `bind` set one.
    pub fn say(&self, line: &str) {
        self.sink.emit(&self.tagged(line));
    }

    /// The line as it is written: tagged if `bind` set a tag.
    fn tagged(&self, line: &str) -> String {
        self.tag
            .as_ref()
            .map_or_else(|| line.to_string(), |tag| format!("[{tag}] {line}"))
    }

    /// A line that a human must see even in `Quiet`.
    ///
    /// Not a failure — a `Halt` travels as a value. What passes here is what
    /// *does not stop* the run and yet would be a bad surprise later: a kept
    /// workspace, local work missing from the clone.
    pub fn warn(&self, line: &str) {
        self.say(&format!("warning: {line}"));
    }

    /// A line of interest only to autopsy — shown only in `Verbose`, **kept
    /// either way**.
    ///
    /// The autopsy is exactly what these lines are for: dropping them from the
    /// file unless someone thought to ask for `--verbose` in advance means the
    /// one run worth explaining is the one that explains nothing. The console
    /// stays quiet; see [`Sink::keep`].
    pub fn debug(&self, line: &str) {
        if self.verbosity == Verbosity::Verbose {
            self.say(line);
        } else {
            self.sink.keep(&self.tagged(line));
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

    /// A console: it shows, it keeps nothing.
    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }

        fn keep(&self, _line: &str) {}
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

    /// A sink that keeps and shows in two places, as a log file beside a
    /// console does.
    #[derive(Default)]
    struct Shelf {
        shown: RefCell<Vec<String>>,
        kept: RefCell<Vec<String>>,
    }

    impl Sink for Shelf {
        fn emit(&self, line: &str) {
            self.shown.borrow_mut().push(line.to_string());
            self.kept.borrow_mut().push(line.to_string());
        }

        fn keep(&self, line: &str) {
            self.kept.borrow_mut().push(line.to_string());
        }
    }

    #[test]
    fn a_debug_line_the_console_hides_is_still_kept() {
        // The failure this answers: the one run worth explaining is the one
        // nobody thought to launch with `--verbose`.
        let shelf = Rc::new(Shelf::default());
        Logbook::new(Rc::clone(&shelf) as Rc<dyn Sink>, Verbosity::Normal).debug("saw: 3 tasks");
        assert!(shelf.shown.borrow().is_empty());
        assert_eq!(shelf.kept.borrow()[0], "saw: 3 tasks");
    }

    #[test]
    fn a_kept_line_carries_the_tag_the_shown_one_would_have() {
        let shelf = Rc::new(Shelf::default());
        Logbook::new(Rc::clone(&shelf) as Rc<dyn Sink>, Verbosity::Normal)
            .bind("r3")
            .debug("saw: 3 tasks");
        assert_eq!(shelf.kept.borrow()[0], "[r3] saw: 3 tasks");
    }
}
