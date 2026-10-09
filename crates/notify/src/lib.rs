#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]

//! What the plant should tell a human, and how loud.
//!
//! Notifications come from two sources, and only two:
//!
//! - **the harness's own events** (`harness_core::traces::Event`), recorded
//!   once where they happen — a lane that stopped for a human, a task that
//!   keeps failing, a quota run out, a watch that cannot read the board;
//! - **a few observed facts** the harness cannot tell, because nothing of it
//!   is there when they happen — a watch that is no longer running, an issue
//!   a human labelled, a subscription window nearly spent.
//!
//! This crate only decides: it reads no file and calls nothing. A reader
//! (the view) hands it the events and the facts, and shows what comes back.
//! The same decision can later feed a desktop notification or a chat message
//! without being written twice.

pub mod facts;
pub mod feed;
pub mod notification;
pub mod rules;

pub use facts::{Facts, Waiting, waiting_on_a_human};
pub use feed::feed;
pub use notification::{Level, Link, Notification};
