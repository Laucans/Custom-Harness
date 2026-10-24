//! The port a sweep needs: the product's database server.

use std::rc::Rc;

use harness_core::ports::shell::database::Databases;

/// The ports of a sweep.
pub struct Ports {
    /// The databases the factory's role owns, and their drop.
    pub databases: Rc<dyn Databases>,
}
