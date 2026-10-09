//! A notification: what to say, how loud, and where it leads.

use serde::Serialize;

/// How loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Worth knowing; nothing to do.
    Info,
    /// A human has something to do.
    Warning,
    /// Something is broken or blocked.
    Error,
}

/// Where a notification leads when it is opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Link {
    /// An issue of the board.
    Issue {
        /// Its number.
        number: u64,
    },
    /// A screen of the control room, around an instant when one matters.
    Screen {
        /// `journal`, `errors`, `quota`, `costs`.
        screen: String,
        /// The instant the screen should be read around.
        at: Option<String>,
    },
}

/// One thing to tell, grouped: the same subject happening again counts up
/// rather than piling up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notification {
    /// What it is about, stable across reads: `parked:#15`, `quota`, … —
    /// what a reader remembers as seen or dismissed.
    pub key: String,
    /// How loud.
    pub level: Level,
    /// One line.
    pub title: String,
    /// What happened, and what to do when there is something to do.
    pub detail: String,
    /// When it first happened, as the traces write a clock.
    pub first_at: String,
    /// When it last happened.
    pub at: String,
    /// How many times.
    pub count: u32,
    /// Where it leads.
    pub link: Option<Link>,
}

impl Notification {
    /// A notification that happened once, at `at`.
    #[must_use]
    pub fn once(
        key: impl Into<String>,
        level: Level,
        title: impl Into<String>,
        detail: impl Into<String>,
        at: &str,
        link: Option<Link>,
    ) -> Self {
        Self {
            key: key.into(),
            level,
            title: title.into(),
            detail: detail.into(),
            first_at: at.to_string(),
            at: at.to_string(),
            count: 1,
            link,
        }
    }
}

/// Notifications by key, merging repeats: the latest words, the first time,
/// the count, and the loudest level seen.
#[derive(Debug, Default)]
pub struct Board {
    by_key: Vec<Notification>,
}

impl Board {
    /// Adds `fresh`, or counts it up when its key is already there.
    pub fn add(&mut self, fresh: Notification) {
        if let Some(known) = self.by_key.iter_mut().find(|n| n.key == fresh.key) {
            known.count = known.count.saturating_add(fresh.count);
            known.at = fresh.at;
            known.title = fresh.title;
            known.detail = fresh.detail;
            known.level = known.level.max(fresh.level);
            known.link = fresh.link.or_else(|| known.link.take());
        } else {
            self.by_key.push(fresh);
        }
    }

    /// Forgets `key`: what it said is over.
    pub fn clear(&mut self, key: &str) {
        self.by_key.retain(|n| n.key != key);
    }

    /// The notification under `key`, if any.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Notification> {
        self.by_key.iter().find(|n| n.key == key)
    }

    /// Everything, loudest first, then newest first.
    #[must_use]
    pub fn into_sorted(mut self) -> Vec<Notification> {
        self.by_key
            .sort_by(|a, b| b.level.cmp(&a.level).then_with(|| b.at.cmp(&a.at)));
        self.by_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeat_counts_up_keeps_the_first_time_and_the_loudest_level() {
        let mut board = Board::default();
        board.add(Notification::once("k", Level::Warning, "a", "", "t1", None));
        board.add(Notification::once("k", Level::Error, "b", "", "t2", None));
        board.add(Notification::once("k", Level::Info, "c", "", "t3", None));
        let all = board.into_sorted();
        assert_eq!(all.len(), 1);
        let one = &all[0];
        assert_eq!((one.count, one.level), (3, Level::Error));
        assert_eq!(
            (one.first_at.as_str(), one.at.as_str(), one.title.as_str()),
            ("t1", "t3", "c")
        );
    }

    #[test]
    fn the_loudest_come_first_then_the_newest() {
        let mut board = Board::default();
        board.add(Notification::once("a", Level::Info, "", "", "t9", None));
        board.add(Notification::once("b", Level::Error, "", "", "t1", None));
        board.add(Notification::once("c", Level::Error, "", "", "t5", None));
        let keys: Vec<String> = board.into_sorted().into_iter().map(|n| n.key).collect();
        assert_eq!(keys, ["c", "b", "a"]);
    }
}
