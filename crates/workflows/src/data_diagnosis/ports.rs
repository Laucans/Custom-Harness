//! The ports a diagnosis needs: a paid session, and where it is billed.

use std::rc::Rc;

use harness_core::ports::agent::SessionFactory;
use harness_core::ports::store::spending::Spending;

/// The ports of a diagnosis.
pub struct Ports {
    /// What opens the paid session — or runs it dry.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where its spending is recorded.
    pub spending: Rc<dyn Spending>,
}
