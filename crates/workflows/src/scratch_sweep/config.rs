//! What a sweep needs to know, by value.

use std::time::Duration;

/// The least time between two sweeps, in seconds.
pub const EVERY: u64 = 600;

/// The age past which an idle scratch database is stale, in seconds: no
/// test holds a database it never connects to for half an hour.
pub const OLDER_THAN: u64 = 1800;

/// The most databases one sweep drops: each unlinks a few megabytes, and
/// the watch waits for them. A backlog is worked off a sweep at a time.
pub const MOST: usize = 200;

/// How long one sweep's drops may take, whatever the server does: a hung
/// server holds the watch's loop — its tick, its stop — this long, no more.
pub const DROPS_WITHIN: Duration = Duration::from_secs(60);

/// The settings of a sweep.
pub struct Config {
    /// The least time between two sweeps, in seconds.
    pub every: u64,
    /// The age past which an idle scratch database is dropped, in seconds.
    pub older_than: u64,
    /// The databases never dropped whatever their name: the ones
    /// `DATABASE_URL` and `DATABASE_URL_READONLY` name.
    pub keep: Vec<String>,
    /// The most databases one sweep drops.
    pub most: usize,
    /// How long one sweep's drops may take.
    pub within: Duration,
}

impl Config {
    /// The sweep's settings, keeping `keep`.
    #[must_use]
    pub const fn new(keep: Vec<String>) -> Self {
        Self {
            every: EVERY,
            older_than: OLDER_THAN,
            keep,
            most: MOST,
            within: DROPS_WITHIN,
        }
    }
}
