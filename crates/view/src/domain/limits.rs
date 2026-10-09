//! The rate limits the page shows, read when it asks: Claude's subscription
//! windows and the GitHub API buckets, each with how it was read.
//!
//! Claude's windows are only known through a session: every session's
//! stream carries a `rate_limit_event`. The runs keep the last one they saw
//! (`quota.json`), but only once a session ends, so during a long session
//! the kept reading lags. A fresh one costs a minimal session — a fraction
//! of a cent — so it is cached for [`CLAUDE_FRESH_SECS`] and not paid on
//! every click. GitHub answers `rate_limit` for free, every time.

use harness_core::domain::quota::Reading;
use serde::Serialize;
use serde_json::Value;

use crate::ports::GithubWindow;

/// How long a probed Claude reading is good enough for an opened panel.
pub const CLAUDE_FRESH_SECS: u64 = 60;

/// How long it is good enough when the human asked to read again.
pub const CLAUDE_FORCED_SECS: u64 = 10;

/// The buckets the harness spends — the rest are listed only once used.
const GITHUB_SPENT: [&str; 3] = ["core", "graphql", "search"];

/// One source's answer: what it said, or why it said nothing, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Read<T> {
    /// The answer, when there was one.
    pub value: Option<T>,
    /// Why there was none.
    pub error: Option<String>,
    /// When it was read, in seconds since the epoch.
    pub at: u64,
}

impl<T> Read<T> {
    /// The answer of a read made at `at`.
    pub fn of(answer: Result<T, String>, at: u64) -> Self {
        match answer {
            Ok(value) => Self {
                value: Some(value),
                error: None,
                at,
            },
            Err(error) => Self {
                value: None,
                error: Some(error),
                at,
            },
        }
    }
}

/// Everything the rate-limits screen shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Claude's subscription windows.
    pub claude: Read<Reading>,
    /// The GitHub API buckets.
    pub github: Read<Vec<GithubWindow>>,
}

/// Whether a Claude reading made at `read_at` must be read again at `now`.
#[must_use]
pub const fn claude_stale(read_at: Option<u64>, now: u64, forced: bool) -> bool {
    let fresh = if forced {
        CLAUDE_FORCED_SECS
    } else {
        CLAUDE_FRESH_SECS
    };
    match read_at {
        Some(at) => now.saturating_sub(at) >= fresh,
        None => true,
    }
}

/// The buckets of a `gh api rate_limit` answer: the ones the harness spends,
/// then any other already in use, by name.
///
/// # Errors
///
/// The answer is not the JSON GitHub sends.
pub fn github_windows(json: &str) -> Result<Vec<GithubWindow>, String> {
    let answer: Value =
        serde_json::from_str(json).map_err(|e| format!("not a rate_limit answer: {e}"))?;
    let resources = answer
        .get("resources")
        .and_then(Value::as_object)
        .ok_or_else(|| "the answer has no resources".to_string())?;
    let number = |bucket: &Value, key: &str| bucket.get(key).and_then(Value::as_u64).unwrap_or(0);
    let mut windows: Vec<GithubWindow> = resources
        .iter()
        .map(|(name, bucket)| GithubWindow {
            name: name.clone(),
            limit: number(bucket, "limit"),
            used: number(bucket, "used"),
            resets_at: number(bucket, "reset"),
        })
        .filter(|w| GITHUB_SPENT.contains(&w.name.as_str()) || w.used > 0)
        .collect();
    windows.sort_by_key(|w| {
        (
            GITHUB_SPENT
                .iter()
                .position(|name| *name == w.name)
                .unwrap_or(GITHUB_SPENT.len()),
            w.name.clone(),
        )
    });
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const ANSWER: &str = r#"{"resources":{
        "search":{"limit":30,"used":2,"remaining":28,"reset":1791507096},
        "graphql":{"limit":5000,"used":120,"remaining":4880,"reset":1791510636},
        "core":{"limit":5000,"used":4999,"remaining":1,"reset":1791510636},
        "code_scanning_upload":{"limit":1000,"used":0,"remaining":1000,"reset":1791510636},
        "integration_manifest":{"limit":5000,"used":3,"remaining":4997,"reset":1791510636}
    },"rate":{"limit":5000,"used":4999}}"#;

    #[test]
    fn the_spent_buckets_come_first_then_any_other_in_use() {
        let windows = github_windows(ANSWER).expect("parsed");
        let names: Vec<&str> = windows.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["core", "graphql", "search", "integration_manifest"]);
        assert_eq!(windows[0].used, 4999);
        assert_eq!(windows[0].limit, 5000);
        assert_eq!(windows[0].resets_at, 1_791_510_636);
    }

    #[test]
    fn an_answer_that_is_not_github_s_is_an_error() {
        assert!(github_windows("gh: not logged in").is_err());
        assert!(github_windows("{}").is_err());
    }

    #[test]
    fn a_claude_reading_is_paid_again_only_once_it_aged() {
        assert!(claude_stale(None, 1000, false));
        assert!(!claude_stale(Some(990), 1000, false));
        assert!(claude_stale(Some(900), 1000, false));
        assert!(!claude_stale(Some(995), 1000, true));
        assert!(claude_stale(Some(985), 1000, true));
    }

    #[test]
    fn a_read_keeps_its_answer_or_its_reason() {
        let ok: Read<u8> = Read::of(Ok(3), 7);
        assert_eq!((ok.value, ok.error, ok.at), (Some(3), None, 7));
        let ko: Read<u8> = Read::of(Err("no gh".to_string()), 7);
        assert_eq!(ko.error.as_deref(), Some("no gh"));
    }

    proptest! {
        #[test]
        fn any_answer_parses_without_panicking(text in "\\PC{0,200}") {
            let _ = github_windows(&text);
        }
    }
}
