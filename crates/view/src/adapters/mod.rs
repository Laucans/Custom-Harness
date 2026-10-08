//! The outside, wrapped: the disk the harness writes its traces to, the
//! GitHub the board lives on, the pseudo-terminal the steward works in, and
//! the watch process the page starts and stops.
//! Nothing outside this module and `server` opens a file, a socket or a
//! process.

pub mod claude_bin;
pub mod fs_traces;
pub mod gh_board;
pub mod pty;
pub mod watch_proc;
