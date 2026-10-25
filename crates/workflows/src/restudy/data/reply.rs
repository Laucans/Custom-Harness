//! What a re-study's session answered — read here, with no I/O.
//!
//! Two lines end the reply: `RESTUDY: tasks [<slice>, …]`, every missing
//! unit as a stop declares one (`dev_loop::data::missing::listed`), and
//! `RESTUDY_CONTEST: [{"rule", "test", "why"}, …]`, the rule tests that
//! assert what their rule does not say. Not `AGENT_LOOP_STOP`: that marker
//! is a halt, and the breaker counts it.

use serde::Deserialize;

use crate::common::json_reply;
use crate::dev_loop::data::missing::{self, Asked};

/// The marker of the slices' line.
pub const TASKS: &str = "RESTUDY: tasks";
/// The marker of the contests' line.
pub const CONTEST: &str = "RESTUDY_CONTEST:";
/// The most slices one re-study may declare: a guard against a re-plan of
/// the milestone, not a target.
pub const MAX_RESTUDY_SLICES: usize = 12;

/// A rule test to contest: it asserts what its rule does not say.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Contest {
    /// The rule (`R15`).
    pub rule: String,
    /// Where its test asserts it (`tests/rules/journey/R15.spec.ts:42`).
    #[serde(default)]
    pub test: String,
    /// What the test asserts that the rule does not say.
    #[serde(default)]
    pub why: String,
}

/// What a re-study found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Studied {
    /// Every slice declared, judged as a stop's are.
    pub asked: Vec<Asked>,
    /// The rule tests to contest.
    pub contests: Vec<Contest>,
}

/// The last line that says `marker`, past its indentation and Markdown:
/// where the line starts, and where its words after the marker start.
fn marked(text: &str, marker: &str) -> Option<(usize, usize)> {
    let mut found = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(|c: char| {
            c.is_whitespace() || matches!(c, '*' | '_' | '>' | '`' | '#')
        });
        if let Some(after) = trimmed.strip_prefix(marker) {
            let words = offset + line.len() - after.len();
            found = Some((offset, words));
        }
        offset += line.len();
    }
    found
}

/// The words after `marker`'s line, to the `other` marker's line when it
/// comes after, else to the end: a list may span several lines, whichever
/// of the two comes first.
fn span<'a>(text: &'a str, marker: &str, other: &str) -> Option<&'a str> {
    let (_, words) = marked(text, marker)?;
    let rest = text.get(words..).unwrap_or_default();
    Some(
        marked(rest, other)
            .and_then(|(line, _)| rest.get(..line))
            .unwrap_or(rest)
            .trim(),
    )
}

/// What the session answered, read.
///
/// # Errors
/// No `RESTUDY: tasks` line, a list that does not read or names more than
/// [`MAX_RESTUDY_SLICES`] slices, a contest list that does not read — in
/// words the session acts on when it is asked once more.
pub fn parse(text: &str) -> Result<Studied, String> {
    let Some(said) = span(text, TASKS, CONTEST) else {
        return Err(format!(
            "no `{TASKS} [...]` line ends the reply (`{TASKS} []` when nothing is missing)"
        ));
    };
    let slices: Vec<serde_json::Value> = json_reply::parse_array(said)
        .map_err(|error| format!("the `{TASKS}` list does not read ({error})"))?;
    if slices.len() > MAX_RESTUDY_SLICES {
        return Err(format!(
            "the `{TASKS}` list names {} slices — at most {MAX_RESTUDY_SLICES}: merge the \
             smallest into their neighbours",
            slices.len()
        ));
    }
    let asked = if slices.is_empty() {
        Vec::new()
    } else {
        missing::listed(said, MAX_RESTUDY_SLICES)
    };
    let contests = match span(text, CONTEST, TASKS) {
        None => Vec::new(),
        Some(list) => json_reply::parse_array::<Contest>(list)
            .map_err(|error| format!("the `{CONTEST}` list does not read ({error})"))?
            .into_iter()
            .filter(|contest| !contest.rule.trim().is_empty())
            .collect(),
    };
    Ok(Studied { asked, contests })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slices #219's stop declared at 13:17, as a re-study declares them.
    const ROUND_6: &str = include_str!("../../../tests/fixtures/stop-219-round-6.txt");

    fn slices() -> &'static str {
        ROUND_6
            .trim()
            .strip_prefix("AGENT_LOOP_STOP: tasks ")
            .expect("a list")
    }

    #[test]
    fn a_reply_names_its_slices_and_its_contests() {
        let text = format!(
            "I walked R11, R13 and R15.\n\n**{TASKS} {}**\n{CONTEST} [{{\"rule\": \"R15\", \
             \"test\": \"tests/rules/journey/R15.spec.ts:42\", \"why\": \"the position depends \
             on the scroll\"}}, {{\"rule\": \" \"}}]\n",
            slices()
        );
        let studied = parse(&text).expect("reads");
        assert_eq!(studied.asked.len(), 2);
        assert!(studied.asked.iter().all(|a| matches!(a, Asked::Task(_))));
        assert_eq!(
            studied.contests,
            [Contest {
                rule: "R15".to_string(),
                test: "tests/rules/journey/R15.spec.ts:42".to_string(),
                why: "the position depends on the scroll".to_string(),
            }]
        );
    }

    #[test]
    fn a_contest_list_pretty_printed_over_several_lines_reads() {
        let contest = "RESTUDY_CONTEST: [\n  {\n    \"rule\": \"R15\",\n    \"test\": \
                       \"tests/rules/journey/R15.spec.ts:42\",\n    \"why\": \"scroll\"\n  \
                       },\n  {\"rule\": \"R16\"}\n]\n";
        let after = format!("Walked.\nRESTUDY: tasks {}\n{contest}", slices());
        let studied = parse(&after).expect("reads");
        assert_eq!(studied.asked.len(), 2);
        let rules: Vec<&str> = studied.contests.iter().map(|c| c.rule.as_str()).collect();
        assert_eq!(rules, ["R15", "R16"]);
        assert_eq!(studied.contests[0].why, "scroll");
        // The contest first, over several lines: it runs to the slices' line.
        let before = format!("Walked.\n{contest}RESTUDY: tasks {}\n", slices());
        let studied = parse(&before).expect("reads");
        assert_eq!(studied.asked.len(), 2);
        assert_eq!(studied.contests.len(), 2);
    }

    #[test]
    fn nothing_missing_is_an_empty_list_and_no_contest_line_is_none() {
        let studied = parse("All built.\nRESTUDY: tasks []\n").expect("reads");
        assert!(studied.asked.is_empty() && studied.contests.is_empty());
        let contested = parse("RESTUDY: tasks []\nRESTUDY_CONTEST: []").expect("reads");
        assert!(contested.contests.is_empty(), "{:?}", contested.contests);
        // The contest line first: the slices still read to the end.
        let before = format!("RESTUDY_CONTEST: []\nRESTUDY: tasks {}", slices());
        assert_eq!(parse(&before).expect("reads").asked.len(), 2);
    }

    #[test]
    fn a_reply_without_its_line_or_with_one_that_does_not_read_is_asked_again() {
        assert!(
            parse("I found three gaps.")
                .expect_err("no line")
                .contains("no `RESTUDY: tasks")
        );
        assert!(
            parse("AGENT_LOOP_STOP: tasks []").is_err(),
            "a stop is no re-study"
        );
        assert!(
            parse("RESTUDY: tasks the fixtures")
                .expect_err("prose")
                .contains("does not read")
        );
        assert!(
            parse("RESTUDY: tasks []\nRESTUDY_CONTEST: R15 is wrong")
                .expect_err("prose")
                .contains("RESTUDY_CONTEST")
        );
        // The marker alone, its list missing.
        assert!(
            parse("I walked them.\nRESTUDY: tasks")
                .expect_err("no list")
                .contains("does not read")
        );
        // A marker mid-sentence is no line of its own.
        assert!(parse("I will end with RESTUDY: tasks [] as asked").is_err());
    }

    #[test]
    fn thirteen_slices_are_too_many_and_a_slice_declared_wrong_is_refused() {
        let one = r#"{"title": "T", "branch": "feat/t", "brief": "b", "unit": "data-capability", "system": "s", "produces": ["s.t"], "goal": "g", "acceptance": ["given a, when b, then c"]}"#;
        let many: Vec<String> = (0..13)
            .map(|k| one.replace("\"T\"", &format!("\"T{k}\"")))
            .collect();
        let text = format!("RESTUDY: tasks [{}]", many.join(", "));
        assert!(
            parse(&text)
                .expect_err("too many")
                .contains("13 slices — at most 12")
        );
        let twelve = format!("RESTUDY: tasks [{}]", many[..12].join(", "));
        assert_eq!(parse(&twelve).expect("reads").asked.len(), 12);
        let wrong = format!(
            "RESTUDY: tasks [{}]",
            one.replace("data-capability", "widget")
        );
        assert!(matches!(
            parse(&wrong).expect("reads").asked.as_slice(),
            [Asked::Refused { .. }]
        ));
    }

    proptest::proptest! {
        #[test]
        fn reading_any_reply_never_panics(text in "(?s).{0,400}") {
            let _ = parse(&text);
            let _ = parse(&format!("RESTUDY: tasks {text}"));
            let _ = parse(&format!("RESTUDY: tasks []\nRESTUDY_CONTEST: {text}"));
        }
    }
}
