//! The carriers that open an agent session.
//!
//! The port those implement is [`ports::agent`](crate::ports::agent): nothing
//! here is named by a workflow, and nothing here decides — a carrier
//! translates, it does not choose when to pay.

pub mod claude_cli;
pub mod rehearsal;
pub mod stream_log;
