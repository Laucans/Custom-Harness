//! The ports a draft needs: GitHub, `git`, the disk and a process for the
//! clone and the dump, a paid session, its spending, the clock.

use std::rc::Rc;

use harness_core::ports::agent::SessionFactory;
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repos;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::shell::process::Process;
use harness_core::ports::store::spending::Spending;

/// The ports of a draft.
pub struct Ports {
    /// The branch's head, and where the pull request is opened.
    pub gh: Rc<dyn GitHub>,
    /// `git`, on the draft clone.
    pub repos: Rc<dyn Repos>,
    /// The clone's files: the dump, the model.
    pub disk: Rc<dyn Disk>,
    /// Runs the dump script.
    pub process: Rc<dyn Process>,
    /// What opens the paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where its spending is recorded.
    pub spending: Rc<dyn Spending>,
    /// The injected clock: ISO-8601 UTC, to the second.
    pub now: fn() -> String,
}
