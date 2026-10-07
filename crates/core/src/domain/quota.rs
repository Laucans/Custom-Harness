//! What is left of the rate-limit windows, and whether that is enough to start.
//!
//! # Why a reserve at all
//!
//! Measured on milestone 17, task #64: a window ran out in the middle of a
//! `code` stage. Nothing was lost — the branch was already pushed — but the
//! session was cut mid-reasoning, and the run that picked it up again had to
//! work out what the first one had left behind. That cost **4,32 $ and 60
//! turns, delivered nothing**, and the stage then ran a second time for 5,25 $.
//!
//! The retries themselves were free: 125 refusals at 0,0000 $, since a refused
//! session is billed nothing. **The expense is never the refusal, it is the
//! interruption** — a long stage started on a window that cannot hold it.
//!
//! # Two windows, not one
//!
//! The real stream reports several at once, each with its own reset:
//!
//! ```text
//! unifiedWindows: { five_hour: { utilization: 0.4, resetsAt: … },
//!                   seven_day: { utilization: 0.7, resetsAt: … } }
//! ```
//!
//! So the question is not "how full is the window" but "is **any** window nearly
//! gone": the tightest one is what will stop a session, and a five-hour window
//! at 40% used is no comfort when the weekly one is at 97%.
//!
//! # Why `resetsAt` and not a window-length table
//!
//! Each window says when it resets. A reading whose window has since reset says
//! nothing about now, and the timestamp settles that exactly — where guessing
//! `"five_hour means five hours"` would be a table to maintain, and a window name
//! nobody added to it would fall through silently.
//!
//! # Why it fails open
//!
//! Every branch that is not "a window is demonstrably nearly gone" allows the
//! run: no reading, an unreadable one, one whose windows have all reset. A gate
//! that blocked on *absence* would be the deadlock the doctor exists to prevent —
//! the loop stops, and the only thing that could produce a fresh reading is the
//! run the gate is refusing.

/// How much of a window must remain for a dev phase to start, in percent.
///
/// Ten percent of a five-hour window is about thirty minutes, longer than any
/// single stage measured so far — the longest was `code` on #64 at 81 turns.
/// Below it, a stage is likely to be cut rather than finish.
///
/// **Whole percent, compared as an integer.** A float comparison against `0.10`
/// refuses a window sitting exactly on the floor: `1.0 - 0.90` is
/// `0.09999999999999998`. A test caught it, and counting in the unit the message
/// speaks in beats an epsilon nobody can read.
pub const RESERVE_PERCENT: u32 = 10;

/// What to call a window the stream does not name.
///
/// Shared with the reader in `adapters::agent::stream_log`, so the name a
/// reading carries and the name a log line shows cannot drift apart.
pub const UNNAMED: &str = "window";

/// One rate-limit window, as the stream reports it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Window {
    /// `five_hour`, `seven_day`, … — what the stream calls it.
    pub name: String,
    /// How much of it is gone, `0.0..=1.0`.
    pub utilization: f64,
    /// When it resets, in seconds since the epoch. `0` when the stream did not
    /// say, which [`Window::has_reset`] then treats as "no known reset".
    pub resets_at: u64,
}

impl Window {
    /// How much of this window is left, `0.0..=1.0`.
    #[must_use]
    pub fn left(&self) -> f64 {
        (1.0 - self.utilization).clamp(0.0, 1.0)
    }

    /// The same, as whole percent — the unit the decision is taken in.
    //
    // The cast cannot truncate or lose a sign: `left` clamps to `0.0..=1.0`, so
    // the rounded product lies in `0..=100`. The narrow allow is the honest
    // alternative to a `try_from` whose error arm would be unreachable and
    // therefore untestable.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "left() clamps to 0.0..=1.0, so the product is 0..=100"
    )]
    pub fn left_percent(&self) -> u32 {
        (self.left() * 100.0).round() as u32
    }

    /// Whether this window has reset since it was read, and so says nothing.
    ///
    /// A missing reset time (`0`) never counts as reset: it is the stream's
    /// silence, not a timestamp in the past, and discarding a reading on it would
    /// throw away the only measurement available.
    #[must_use]
    pub const fn has_reset(&self, now: u64) -> bool {
        self.resets_at > 0 && now >= self.resets_at
    }

    /// Whether this window is too nearly spent to start a dev phase on.
    #[must_use]
    pub fn is_too_tight(&self) -> bool {
        self.left_percent() < RESERVE_PERCENT
    }
}

/// The last thing a session's stream said about the rate-limit windows.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reading {
    /// Every window the stream reported, in the order it reported them.
    pub windows: Vec<Window>,
    /// When it was read, in seconds since the epoch.
    ///
    /// Kept for the journal and for an operator reading the file by hand; the
    /// decision itself goes by each window's own `resets_at`.
    pub at: u64,
}

impl Reading {
    /// The windows that have not reset since this was read — the only ones that
    /// still describe now.
    #[must_use]
    pub fn live(&self, now: u64) -> Vec<&Window> {
        self.windows
            .iter()
            .filter(|window| !window.has_reset(now))
            .collect()
    }

    /// The live window with the least left: the one that will stop a session
    /// first, and therefore the one that decides.
    #[must_use]
    pub fn tightest(&self, now: u64) -> Option<&Window> {
        self.live(now)
            .into_iter()
            .min_by_key(|window| window.left_percent())
    }
}

/// What a preflight concludes about starting a dev phase now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Start. The reason is for the journal, not for a human to act on.
    Start(String),
    /// Do not start, and this is what to tell the human.
    Wait(String),
}

/// Whether a dev phase should start, given what was last read.
///
/// `None` starts: see the module's note on failing open.
#[must_use]
pub fn decide(reading: Option<&Reading>, now: u64) -> Verdict {
    let Some(reading) = reading else {
        return Verdict::Start(
            "no rate-limit reading yet — starting, and this run will leave one".to_string(),
        );
    };
    let Some(tightest) = reading.tightest(now) else {
        return Verdict::Start(
            "every window in the last rate-limit reading has since reset — starting".to_string(),
        );
    };
    if tightest.is_too_tight() {
        return Verdict::Wait(format!(
            "only {}% of the {} window is left, under the {RESERVE_PERCENT}% a \
             dev phase needs. A stage started now would be cut mid-reasoning, \
             and the run that resumed it would redo the thinking — on #64 that \
             cost 4,32 $ for nothing. Waiting for the window to reset costs \
             nothing. Use --ignore-quota to start anyway.",
            tightest.left_percent(),
            tightest.name,
        ));
    }
    Verdict::Start(format!(
        "{}% left of {}, the tightest of {} live window(s)",
        tightest.left_percent(),
        tightest.name,
        reading.live(now).len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000;
    const LATER: u64 = NOW + 60 * 60;

    fn window(name: &str, utilization: f64, resets_at: u64) -> Window {
        Window {
            name: name.to_string(),
            utilization,
            resets_at,
        }
    }

    fn reading(windows: Vec<Window>) -> Reading {
        Reading { windows, at: NOW }
    }

    #[test]
    fn a_fresh_window_starts() {
        let said = decide(Some(&reading(vec![window("five_hour", 0.20, LATER)])), NOW);
        let Verdict::Start(why) = said else {
            panic!("must start")
        };
        assert!(why.contains("80%"), "{why}");
    }

    #[test]
    fn an_almost_exhausted_window_waits() {
        let said = decide(Some(&reading(vec![window("five_hour", 0.95, LATER)])), NOW);
        let Verdict::Wait(why) = said else {
            panic!("must wait")
        };
        assert!(why.contains("5%"), "{why}");
        assert!(why.contains("10%"), "the floor is named: {why}");
        // The escape hatch is named: the gate is a default, not a verdict about
        // the human's judgement.
        assert!(why.contains("--ignore-quota"), "{why}");
    }

    #[test]
    fn the_tightest_window_decides_even_when_another_is_wide_open() {
        // The case the real nested shape revealed: a five-hour window at 40%
        // used is no comfort when the weekly one is nearly gone.
        let said = decide(
            Some(&reading(vec![
                window("five_hour", 0.40, LATER),
                window("seven_day", 0.97, LATER),
            ])),
            NOW,
        );
        let Verdict::Wait(why) = said else {
            panic!("the weekly window must decide")
        };
        assert!(why.contains("seven_day"), "{why}");
        assert!(why.contains("3%"), "{why}");
    }

    #[test]
    fn exactly_the_reserve_still_starts() {
        // The floor is "less than", so a window sitting on it is enough. An
        // off-by-one here would stop a loop that had the room it needed.
        let said = decide(Some(&reading(vec![window("five_hour", 0.90, LATER)])), NOW);
        assert!(matches!(said, Verdict::Start(_)), "{said:?}");
    }

    #[test]
    fn no_reading_starts_rather_than_deadlocking() {
        // The deadlock this avoids: only a run can produce a reading, and the
        // gate would be refusing the run.
        let Verdict::Start(why) = decide(None, NOW) else {
            panic!("absence must not block")
        };
        assert!(why.contains("no rate-limit reading"), "{why}");
    }

    #[test]
    fn a_window_past_its_reset_is_ignored() {
        let spent = reading(vec![window("five_hour", 1.0, NOW + 10)]);
        assert!(matches!(decide(Some(&spent), NOW), Verdict::Wait(_)));
        // Ten seconds later it has reset, and says nothing about now.
        assert!(matches!(decide(Some(&spent), NOW + 10), Verdict::Start(_)));
    }

    #[test]
    fn a_reading_whose_windows_have_all_reset_starts() {
        let old = reading(vec![
            window("five_hour", 1.0, NOW),
            window("seven_day", 1.0, NOW),
        ]);
        let Verdict::Start(why) = decide(Some(&old), NOW + 1) else {
            panic!("all reset must start")
        };
        assert!(why.contains("since reset"), "{why}");
    }

    #[test]
    fn a_window_with_no_reset_time_is_believed_rather_than_discarded() {
        // `0` is the stream's silence, not a timestamp in the past. Treating it
        // as reset would throw away the only measurement available.
        let quiet = reading(vec![window(UNNAMED, 0.99, 0)]);
        assert!(!quiet.windows[0].has_reset(NOW + 10_000_000));
        assert!(matches!(decide(Some(&quiet), NOW), Verdict::Wait(_)));
    }

    #[test]
    fn a_reset_window_does_not_mask_a_live_one_that_is_tight() {
        let mixed = reading(vec![
            window("five_hour", 0.10, NOW), // already reset, 90% left
            window("seven_day", 0.98, LATER),
        ]);
        let Verdict::Wait(why) = decide(Some(&mixed), NOW + 1) else {
            panic!("the live window must still decide")
        };
        assert!(why.contains("seven_day"), "{why}");
    }

    #[test]
    fn utilization_outside_the_scale_does_not_produce_a_nonsense_percentage() {
        // The stream is not ours to trust blindly: it reports a float.
        assert_eq!(window("w", 1.4, 0).left_percent(), 0);
        assert_eq!(window("w", -0.5, 0).left_percent(), 100);
    }
}
