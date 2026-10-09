//! The outside, wrapped: the disk the harness writes its traces to, the
//! GitHub the board lives on, the pseudo-terminal the steward works in, the
//! watch process the page starts and stops, and the CLIs that say how much
//! of each rate limit is left, and the yard The janitor weighs and sweeps.
//! Nothing outside this module and `server` opens a file, a socket or a
//! process.

pub mod claude_bin;
pub mod fs_notes;
pub mod fs_traces;
pub mod fs_yard;
pub mod gh_board;
pub mod limits_cli;
pub mod product;
pub mod pty;
pub mod watch_proc;
