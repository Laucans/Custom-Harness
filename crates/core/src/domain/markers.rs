//! The verbal contract: what a session says so we know where it stands.
//!
//! A session without `AGENT_LOOP_OK` falls back to structural checks — the
//! gates. The marker is not proof that work is done, it is the short version;
//! a merged PR is proof.

/// The session states what it did.
pub const OK: &str = "AGENT_LOOP_OK";

/// The session stops itself. A correct result, not a failure.
pub const STOP: &str = "AGENT_LOOP_STOP";

/// The line bearing [`STOP`], if the text contains one.
///
/// Returned whole rather than as a boolean: the reason the session gives is
/// on this line, and it is what a human will read in the journal.
#[must_use]
pub fn stop_line(text: &str) -> Option<String> {
    marked(text, STOP)
}

/// The line bearing [`OK`], if the text contains one.
///
/// Returned for the same reason as [`stop_line`]: it is the summary the
/// session gives of itself, and it is this line that a journal picks up. Its
/// absence is not a failure — it only falls back to structural checks.
#[must_use]
pub fn ok_line(text: &str) -> Option<String> {
    marked(text, OK)
}

fn marked(text: &str, marker: &str) -> Option<String> {
    text.lines()
        .find(|line| line.contains(marker))
        .map(|line| line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_without_the_marker_has_no_stop_line() {
        assert!(stop_line("I'm done, everything is fine").is_none());
    }

    #[test]
    fn the_whole_line_comes_back_not_just_the_marker() {
        let text = "here is what I did\nAGENT_LOOP_STOP: the SPEC is empty\nend";
        assert_eq!(
            stop_line(text).as_deref(),
            Some("AGENT_LOOP_STOP: the SPEC is empty")
        );
    }

    #[test]
    fn the_line_is_trimmed_so_indentation_does_not_leak_into_the_journal() {
        assert_eq!(
            stop_line("   AGENT_LOOP_STOP: nothing to do   ").as_deref(),
            Some("AGENT_LOOP_STOP: nothing to do")
        );
    }

    #[test]
    fn ok_and_stop_are_distinct_markers() {
        assert!(stop_line("AGENT_LOOP_OK: delivered").is_none());
        assert!(ok_line("AGENT_LOOP_STOP: blocked").is_none());
    }

    #[test]
    fn the_ok_line_comes_back_whole_like_the_stop_line() {
        let text = "I did this\nAGENT_LOOP_OK: grid delivered\n";
        assert_eq!(
            ok_line(text).as_deref(),
            Some("AGENT_LOOP_OK: grid delivered")
        );
    }
}
