//! The register of relays, one JSON file per key under the factory's
//! `.llocal/handoffs/` — implementation of [`Handoffs`].
//!
//! A relay is taken by renaming its file away: of two lanes asking at once,
//! one rename succeeds, the other finds nothing. A relay older than its
//! session's cache (an hour, kept short of it) is not worth branching from.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::store::handoffs::{Handoff, Handoffs};

/// How long a relay stays worth taking: short of the cache's hour.
const FRESH: Duration = Duration::from_mins(50);

/// The relays, under one folder.
pub struct FsHandoffs {
    dir: PathBuf,
}

impl FsHandoffs {
    /// The relays kept under `dir`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// The file of `key`, its name kept to what a path holds.
    fn file(&self, key: &str) -> PathBuf {
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{safe}.json"))
    }
}

/// Whether a relay left at `at` is too old to take at `now`, in seconds.
const fn stale(at: u64, now: u64) -> bool {
    now.saturating_sub(at) > FRESH.as_secs()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Handoffs for FsHandoffs {
    fn leave(&self, key: &str, handoff: &Handoff) -> Outcome<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| Halt::Failed(format!("handoffs: {e}")))?;
        let text = serde_json::json!({
            "session": handoff.session,
            "cost_usd": handoff.cost_usd,
            "task": handoff.task,
            "context_tokens": handoff.context_tokens,
            "at": now(),
        })
        .to_string();
        std::fs::write(self.file(key), text).map_err(|e| Halt::Failed(format!("handoffs: {e}")))
    }

    fn take(&self, key: &str) -> Outcome<Option<Handoff>> {
        let file = self.file(key);
        let taken = file.with_extension(format!("taken-{}", std::process::id()));
        if std::fs::rename(&file, &taken).is_err() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&taken).unwrap_or_default();
        let _ = std::fs::remove_file(&taken);
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Ok(None);
        };
        let at = json
            .get("at")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if stale(at, now()) {
            return Ok(None);
        }
        let Some(session) = json.get("session").and_then(serde_json::Value::as_str) else {
            return Ok(None);
        };
        Ok(Some(Handoff {
            session: session.to_string(),
            cost_usd: json.get("cost_usd").and_then(serde_json::Value::as_f64),
            task: json
                .get("task")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            context_tokens: json
                .get("context_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!("handoffs-{tag}-{}-{nanos}", std::process::id()))
    }

    fn relay() -> Handoff {
        Handoff {
            session: "s-1".to_string(),
            cost_usd: Some(0.4),
            task: "38".to_string(),
            context_tokens: 61_000,
        }
    }

    #[test]
    fn a_relay_is_taken_once_and_as_it_was_left() {
        let at = dir("once");
        let register = FsHandoffs::new(&at);
        assert_eq!(register.take("17-ai").expect("read"), None, "none left yet");
        register.leave("17-ai", &relay()).expect("left");
        assert_eq!(register.take("17-ai").expect("read"), Some(relay()));
        assert_eq!(register.take("17-ai").expect("read"), None, "taken once");
        let _ = std::fs::remove_dir_all(&at);
    }

    #[test]
    fn a_relay_is_fresh_up_to_its_hour_s_margin() {
        let fresh = FRESH.as_secs();
        assert!(!stale(1_000, 1_000 + fresh), "at the margin, still fresh");
        assert!(stale(1_000, 1_000 + fresh + 1));
        assert!(!stale(5_000, 1_000), "a clock that went back is no age");
    }

    #[test]
    fn a_stale_relay_is_not_worth_taking() {
        let at = dir("stale");
        let register = FsHandoffs::new(&at);
        register.leave("17/ai", &relay()).expect("left");
        let file = register.file("17/ai");
        assert!(file.ends_with("17_ai.json"), "{file:?}");
        let old = format!(
            "{{\"session\":\"s-1\",\"task\":\"38\",\"at\":{}}}",
            now() - FRESH.as_secs() - 5
        );
        std::fs::write(&file, old).expect("aged");
        assert_eq!(register.take("17/ai").expect("read"), None);
        let _ = std::fs::remove_dir_all(&at);
    }
}
