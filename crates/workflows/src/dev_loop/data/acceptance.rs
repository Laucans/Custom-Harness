//! A task's acceptance criteria, numbered `AC1..ACn`, and what proves each:
//! a test the PR adds naming it, an existing test the PR body points to, or
//! a line of the PR body saying why none can.
//!
//! One reading for every end of a round: the code prompt and the review's
//! brief number the criteria with [`numbered`], and the loop's check after
//! the session ([`crate::dev_loop::checks::criteria`]) counts them with
//! [`criteria`] — so whatever the parser makes of an odd section, the
//! session, the check and the reviewer agree on what `AC3` is.
//!
//! Why it exists: the blind review of the data layer (2026-10-25) found the
//! one defect left on sonnet's side was a criterion no test checked (#510:
//! AC3 and AC4 never verified).
//!
//! Pure: no I/O.

use std::collections::{BTreeMap, BTreeSet};

use crate::common::sections;

/// The key of the section the criteria are read from.
pub const SECTION: &str = "acceptance-criteria";

/// What a PR body line says of a criterion no test can prove.
pub const UNTESTABLE: &str = "not testable here";

/// What a PR body line says of a criterion an existing test already proves:
/// `AC4: covered by tests/rules/R103.spec.ts — the rule runs it`.
pub const COVERED: &str = "covered by";

/// The longest criterion number read: a longer run of digits is an id or a
/// hash, not a criterion's name.
const NUMBER_DIGITS: usize = 3;

/// A bullet's text without its marker (`-`, `*`, `+`, `1.`, `1)`), or
/// `None` when the line is not a bullet.
fn bullet(line: &str) -> Option<&str> {
    if matches!(line, "-" | "*" | "+") {
        return Some("");
    }
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest);
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = line.get(digits..)?;
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
}

/// A criterion's text without a checkbox (`[ ]`, `[x]`) or a number it was
/// already given (`AC3:`, `**AC3** —`): numbering twice must not nest.
fn bare(text: &str) -> &str {
    let mut text = text.trim();
    for checkbox in ["[ ] ", "[x] ", "[X] "] {
        if let Some(rest) = text.strip_prefix(checkbox) {
            text = rest.trim_start();
        }
    }
    let unbolded = text.strip_prefix("**").unwrap_or(text);
    let Some(after) = unbolded
        .strip_prefix("AC")
        .or_else(|| unbolded.strip_prefix("ac"))
    else {
        return text;
    };
    let digits = after.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return text;
    }
    let rest = after.get(digits..).unwrap_or_default();
    let rest = rest.strip_prefix("**").unwrap_or(rest);
    let rest = rest.trim_start_matches([':', ' ', '\t', '-', '—', '–', '.', ')']);
    if rest.len() == after.len().saturating_sub(digits) && !rest.is_empty() {
        // `AC3x…`: not a number given, a word that starts with the letters.
        return text;
    }
    rest.trim()
}

/// The first word of a line, lowercase, without its punctuation.
fn first_word(line: &str) -> String {
    line.split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// The criteria of an `## Acceptance Criteria` section's text, in order.
///
/// A bullet (`-`, `*`, `+`, `1.`, `1)`) opens a criterion. So does a
/// `Given …` line once the one before already has its given, and any plain
/// line after a blank one. A `When`/`Then`/`And`/`But` line, an indented
/// line, or a plain line right under the previous one continues it — a
/// given/when/then over three lines is one criterion. A line ending with
/// `:` and no bullet is an introduction, never a criterion; sub-headings
/// and code fences are skipped.
#[must_use]
pub fn criteria(section: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut has_given = false;
    let mut after_blank = true;
    for raw in section.lines() {
        let line = raw.trim();
        if line.is_empty() {
            after_blank = true;
            continue;
        }
        if line.starts_with('#') || line.starts_with("```") {
            after_blank = true;
            continue;
        }
        let word = first_word(line);
        let continues = matches!(word.as_str(), "when" | "then" | "and" | "but");
        let indented = raw.starts_with([' ', '\t']);
        if let Some(text) = bullet(line).filter(|_| !indented || found.is_empty()) {
            let text = bare(text);
            if !text.is_empty() {
                has_given = first_word(text) == "given";
                found.push(text.to_string());
            }
        } else if let Some(last) = found.last_mut().filter(|_| {
            continues
                || (word == "given" && !has_given && !after_blank)
                || (word != "given" && (indented || !after_blank))
        }) {
            has_given |= word == "given";
            last.push(' ');
            last.push_str(bullet(line).map_or(line, bare));
        } else if !line.ends_with(':') {
            has_given = word == "given";
            found.push(bare(line).to_string());
        }
        after_blank = false;
    }
    found.retain(|text| !text.trim().is_empty());
    found
}

/// The criteria of a task body, read from its `## Acceptance Criteria`.
#[must_use]
pub fn of_body(body: &str) -> Vec<String> {
    sections::parse(body)
        .get(SECTION)
        .map(|text| criteria(text))
        .unwrap_or_default()
}

/// The criteria as the prompt numbers them: `- AC1: …`, one per line.
#[must_use]
pub fn numbered(found: &[String]) -> String {
    found
        .iter()
        .enumerate()
        .map(|(at, text)| format!("- AC{}: {text}", at + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body with its criteria numbered in place — the body as it was when
/// it has none.
#[must_use]
pub fn numbered_body(body: &str) -> String {
    let found = of_body(body);
    if found.is_empty() {
        return body.to_string();
    }
    sections::replace(body, &[(SECTION, &numbered(&found))])
}

/// How many times a text names each criterion number: every `AC<n>` (any
/// case) standing as a word of its own — `AC2:`, `// AC2, AC3`,
/// `fn ac2_lists_them`.
///
/// A word of its own: no letter or digit right before `ac` nor right after
/// its digits, no hyphen before it (a UUID's `-ac12-` segment), and at most
/// [`NUMBER_DIGITS`] digits (a hex `ac123456`).
#[must_use]
pub fn counted(text: &str) -> BTreeMap<usize, usize> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut found = BTreeMap::new();
    for (start, _) in lower.match_indices("ac") {
        let glued_before = start
            .checked_sub(1)
            .and_then(|i| bytes.get(i))
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-');
        if glued_before {
            continue;
        }
        let from = start.saturating_add(2);
        let digits = bytes
            .get(from..)
            .unwrap_or_default()
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        let to = from.saturating_add(digits);
        let glued_after = bytes.get(to).is_some_and(u8::is_ascii_alphanumeric);
        if digits > NUMBER_DIGITS || glued_after {
            continue;
        }
        if let Some(number) = lower.get(from..to).and_then(|d| d.parse::<usize>().ok()) {
            let count: &mut usize = found.entry(number).or_default();
            *count = count.saturating_add(1);
        }
    }
    found
}

/// The criterion numbers a text names: see [`counted`].
#[must_use]
pub fn named(text: &str) -> BTreeSet<usize> {
    counted(text).into_keys().collect()
}

/// The criterion numbers a change names more often than the text before it
/// did: a file that already named `AC1` for an earlier task covers no
/// later `AC1` unless the change adds one.
#[must_use]
pub fn added(base: &str, head: &str) -> BTreeSet<usize> {
    let before = counted(base);
    counted(head)
        .into_iter()
        .filter(|(number, count)| before.get(number).copied().unwrap_or(0) < *count)
        .map(|(number, _)| number)
        .collect()
}

/// An existing test a PR body points a criterion to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pointer {
    /// The criteria it proves.
    pub criteria: BTreeSet<usize>,
    /// The test file, from the repository's root.
    pub path: String,
    /// The test's name inside it, when the line gives one (`path::name`).
    pub test: Option<String>,
}

/// The criteria a PR body points to an existing test, each with its file.
///
/// A line naming them, then [`COVERED`], the file (`path` or
/// `path::test`), then why — `AC4: covered by tests/rules/R103.spec.ts —
/// the rule runs it`. A line with no reason after the file points nothing.
#[must_use]
pub fn declared_covered(pr_body: &str) -> Vec<Pointer> {
    let quotes = |c: char| matches!(c, '`' | '\'' | '"' | ',' | ';');
    let mut found = Vec::new();
    for line in pr_body.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(at) = lower.find(COVERED) else {
            continue;
        };
        let criteria = named(line.get(..at).unwrap_or_default());
        let rest = line
            .get(at.saturating_add(COVERED.len())..)
            .unwrap_or_default()
            .trim_start();
        let Some(target) = rest.split_whitespace().next() else {
            continue;
        };
        let reason = rest
            .get(target.len()..)
            .unwrap_or_default()
            .trim_start_matches(|c: char| !c.is_alphanumeric());
        if criteria.is_empty() || reason.trim().is_empty() {
            continue;
        }
        let target = target.trim_matches(quotes);
        let (path, test) = target
            .split_once("::")
            .map_or((target, None), |(path, test)| (path, Some(test)));
        let path = path.trim_matches(quotes);
        if !path.chars().any(char::is_alphanumeric) {
            continue;
        }
        found.push(Pointer {
            criteria,
            path: path.to_string(),
            test: test
                .map(|test| test.trim_matches(quotes).to_string())
                .filter(|test| !test.is_empty()),
        });
    }
    found
}

/// True when the file a pointer names, read at the PR's head, is a test
/// file holding the test it names.
#[must_use]
pub fn proves(pointer: &Pointer, text: &str) -> bool {
    is_test_file(&pointer.path, text)
        && pointer
            .test
            .as_ref()
            .is_none_or(|test| text.contains(test.as_str()))
}

/// The criteria a PR body declares no test can prove, each with its reason.
///
/// A line naming them, then [`UNTESTABLE`], then why —
/// `AC3: not testable here — the provider has no sandbox`. A line with no
/// reason after it declares nothing.
#[must_use]
pub fn declared_untestable(pr_body: &str) -> BTreeSet<usize> {
    let mut found = BTreeSet::new();
    for line in pr_body.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(at) = lower.find("not testable") else {
            continue;
        };
        let reason = lower
            .get(at + "not testable".len()..)
            .unwrap_or_default()
            .trim_start();
        let reason = reason.strip_prefix("here").unwrap_or(reason);
        let reason = reason.trim_start_matches(|c: char| !c.is_alphanumeric());
        if reason.trim().is_empty() {
            continue;
        }
        found.extend(named(line.get(..at).unwrap_or_default()));
    }
    found
}

/// Folders and file names that hold tests, whatever the language.
fn test_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let mut folders = lower.split('/').rev().skip(1);
    let name = lower.rsplit('/').next().unwrap_or_default();
    folders.any(|folder| matches!(folder, "tests" | "test" | "__tests__" | "e2e" | "spec"))
        || name.starts_with("test_")
        || ["_test.", ".test.", ".spec.", "_spec.", "_tests.", ".tests."]
            .iter()
            .any(|part| name.contains(part))
}

/// The extensions of source files a test can live in.
const CODE: [&str; 15] = [
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "rb", "java", "kt", "swift", "sh",
    "sql",
];

/// True when the PR's file at `path` may hold a test: a source file.
#[must_use]
pub fn may_hold_tests(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| CODE.contains(&extension.to_lowercase().as_str()))
}

/// What marks a test inside a source file.
const MARKERS: [&str; 14] = [
    "#[test]",
    "#[tokio::test",
    "#[cfg(test)]",
    "proptest!",
    "describe(",
    "it(\"",
    "it('",
    "it(`",
    "test(\"",
    "test('",
    "test(`",
    "def test_",
    "func Test",
    "@Test",
];

/// True when the file is a test file: its path says so, or its text holds a
/// test (a Rust `#[cfg(test)]` module, a `describe(` block).
#[must_use]
pub fn is_test_file(path: &str, text: &str) -> bool {
    test_path(path) || MARKERS.iter().any(|marker| text.contains(marker))
}

/// A file the PR changes, as it was on its base and is at its head.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changed {
    /// From the repository's root.
    pub path: String,
    /// Its text on the base: empty for a file the PR adds.
    pub base: String,
    /// Its text at the PR's head.
    pub head: String,
}

/// The criteria (1-based) neither named by a test file the PR changes more
/// often than before ([`added`]), nor `proven` by an existing test the PR
/// body points to, nor declared untestable in it.
#[must_use]
pub fn uncovered(
    count: usize,
    files: &[Changed],
    pr_body: &str,
    proven: &BTreeSet<usize>,
) -> Vec<usize> {
    let mut covered = declared_untestable(pr_body);
    covered.extend(proven);
    for file in files {
        if is_test_file(&file.path, &file.head) {
            covered.extend(added(&file.base, &file.head));
        }
    }
    (1..=count)
        .filter(|number| !covered.contains(number))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn bullets_of_every_kind_are_one_criterion_each() {
        let section = "- given a, when b, then c\n* second\n+ third\n4. fourth\n5) fifth";
        assert_eq!(
            criteria(section),
            [
                "given a, when b, then c",
                "second",
                "third",
                "fourth",
                "fifth"
            ]
        );
    }

    #[test]
    fn a_given_when_then_over_several_lines_is_one_criterion() {
        let section = "Given a DM\nWhen they open the list\nThen every PJ shows\n\n\
                       Given no PJ\nAnd an empty campaign\nThen the list is empty";
        assert_eq!(
            criteria(section),
            [
                "Given a DM When they open the list Then every PJ shows",
                "Given no PJ And an empty campaign Then the list is empty",
            ]
        );
    }

    #[test]
    fn a_second_given_opens_a_criterion_and_a_scenario_takes_its_first() {
        let section = "Scenario: listed\nGiven a\nThen b\nGiven c\nThen d";
        assert_eq!(
            criteria(section),
            ["Scenario: listed Given a Then b", "Given c Then d"]
        );
        assert_eq!(
            criteria("- given a then b\nGiven c then d\n- Listing\nGiven e\nWhen f"),
            ["given a then b", "Given c then d", "Listing Given e When f"],
            "a bullet's own given counts as the criterion's"
        );
        assert_eq!(
            criteria("Scenario: s\nGiven a\nGiven b"),
            ["Scenario: s Given a", "Given b"],
            "the given a scenario took is its own"
        );
        assert_eq!(
            criteria("Given a\nbut not b\n\nAC12\n- c"),
            ["Given a but not b", "c"],
            "a bare number is no criterion"
        );
    }

    #[test]
    fn a_wrapped_bullet_is_continued_and_an_introduction_is_no_criterion() {
        let section =
            "Done when:\n\n- the list shows\n  every PJ\n- the count\nis right\n\nloose line";
        assert_eq!(
            criteria(section),
            [
                "the list shows every PJ",
                "the count is right",
                "loose line"
            ]
        );
    }

    #[test]
    fn checkboxes_and_numbers_already_given_are_dropped() {
        let section = "- [ ] AC1: first\n- [x] **AC2** — second\n- ACME ships\n- AC3x stays";
        assert_eq!(
            criteria(section),
            ["first", "second", "ACME ships", "AC3x stays"]
        );
    }

    #[test]
    fn sub_headings_fences_and_empty_bullets_are_skipped() {
        let section = "### Happy path\n- one\n```\n- \n```\n-  \n- two";
        assert_eq!(criteria(section), ["one", "two"]);
        assert_eq!(
            criteria("- one\n\nGiven a\nThen b"),
            ["one", "Given a Then b"],
            "a given after a blank line opens its own"
        );
    }

    #[test]
    fn an_indented_bullet_continues_the_one_above() {
        assert_eq!(criteria("- one\n  - detail\n- two"), ["one detail", "two"]);
        assert_eq!(criteria("  - first"), ["first"], "a lone indented bullet");
    }

    #[test]
    fn the_body_s_criteria_are_numbered_in_place() {
        let body =
            "## Business Goal\n\ng\n\n## Acceptance Criteria\n\n- a\n- b\n\n## Technical\n\nt";
        assert_eq!(of_body(body), ["a", "b"]);
        let numbered = numbered_body(body);
        assert!(numbered.contains("- AC1: a\n- AC2: b"), "{numbered}");
        assert!(numbered.contains("## Technical\n\nt"), "{numbered}");
        assert_eq!(of_body(&numbered), ["a", "b"], "numbering is stable");
        let without = "## Business Goal\n\ng";
        assert_eq!(numbered_body(without), without);
        assert!(of_body(without).is_empty(), "no criteria");
    }

    #[test]
    fn a_criterion_is_named_as_a_word_of_its_own() {
        let text = "// AC2: lists\n# ac10 and AC3, AC4\nfn ac5_x() {}\nPAC6 bac7 AC";
        assert_eq!(named(text), BTreeSet::from([2, 3, 4, 5, 10]));
        assert!(named("ACx ac").is_empty());
    }

    #[test]
    fn an_untestable_criterion_counts_only_with_its_reason() {
        let body = "## Plan\n- AC3: not testable here — the provider has no sandbox\n\
                    AC4, AC5: Not testable: needs a GPU\nAC6: not testable here —\n\
                    AC7: not testable here";
        assert_eq!(declared_untestable(body), BTreeSet::from([3, 4, 5]));
    }

    #[test]
    fn a_test_file_is_known_by_its_path_or_by_a_test_inside() {
        for path in [
            "crates/a/tests/engine.rs",
            "tests/rules/x.rs",
            "web/__tests__/a.ts",
            "apps/a/list.test.ts",
            "apps/a/list.spec.tsx",
            "py/test_list.py",
            "go/list_test.go",
            "e2e/flow.ts",
        ] {
            assert!(is_test_file(path, ""), "{path}");
        }
        assert!(is_test_file("src/lib.rs", "#[cfg(test)]\nmod tests {}"));
        assert!(is_test_file("src/a.ts", "describe(\"a\", () => {})"));
        assert!(!is_test_file("src/lib.rs", "// AC2"));
        assert!(!is_test_file("contests/a.rs", ""), "a word ending in test");
    }

    #[test]
    fn only_source_files_may_hold_tests() {
        assert!(may_hold_tests("crates/a/tests/engine.rs"));
        assert!(may_hold_tests("apps/a/list.TSX"));
        assert!(!may_hold_tests("README.md"));
        assert!(!may_hold_tests("data/model.json"));
        assert!(!may_hold_tests("Makefile"));
    }

    fn added_file(path: &str, head: &str) -> Changed {
        Changed {
            path: path.to_string(),
            base: String::new(),
            head: head.to_string(),
        }
    }

    #[test]
    fn a_criterion_is_covered_by_a_test_file_a_pointer_or_a_declaration_only() {
        let files = vec![
            added_file("crates/a/tests/a.rs", "// AC1: x"),
            added_file("crates/a/src/lib.rs", "// AC2 in prod code"),
            added_file("crates/a/src/b.rs", "#[test]\n// AC4: y"),
        ];
        let body = "AC3: not testable here — a real provider";
        let none = BTreeSet::new();
        assert_eq!(uncovered(6, &files, body, &none), [2, 5, 6]);
        assert_eq!(
            uncovered(6, &files, body, &BTreeSet::from([5])),
            [2, 6],
            "proven by an existing test"
        );
        assert!(
            uncovered(0, &files, body, &none).is_empty(),
            "nothing to cover"
        );
    }

    #[test]
    fn names_a_test_file_held_before_the_pr_cover_nothing() {
        let old = "// AC1: old\n#[test]\nfn a() {}\n// AC2, AC3: old";
        let untouched = Changed {
            path: "crates/a/tests/a.rs".to_string(),
            base: old.to_string(),
            head: format!("{old}\n// a line the PR adds"),
        };
        let none = BTreeSet::new();
        assert_eq!(uncovered(3, &[untouched], "", &none), [1, 2, 3]);
        let one_more = Changed {
            path: "crates/a/tests/a.rs".to_string(),
            base: old.to_string(),
            head: format!("{old}\n// AC2: the new one"),
        };
        assert_eq!(uncovered(3, &[one_more], "", &none), [1, 3]);
    }

    #[test]
    fn only_a_name_written_more_often_is_added() {
        assert_eq!(added("", "AC1 AC2"), BTreeSet::from([1, 2]));
        assert_eq!(added("AC1 AC2", "AC1 AC2 AC1"), BTreeSet::from([1]));
        assert!(added("AC1 AC1", "AC1").is_empty(), "one removed");
        assert!(added("AC1", "AC1").is_empty());
    }

    #[test]
    fn a_criterion_name_ends_where_its_digits_do() {
        assert!(named("7c9e6679-ac1e-4f0a-9b1d-ac3b2c1d0e5f").is_empty());
        assert!(named("0b1f-ac2d-11ee").is_empty());
        assert!(named("x-ac12-y ac1234 ac2b").is_empty());
        assert_eq!(
            named("fn ac2_lists_them\n// AC3: x\n(AC4) *AC5* AC6,AC7 ac999"),
            BTreeSet::from([2, 3, 4, 5, 6, 7, 999])
        );
        assert_eq!(counted("AC1 ac1 AC2"), BTreeMap::from([(1, 2), (2, 1)]));
    }

    #[test]
    fn a_pointer_names_its_criteria_its_file_and_why() {
        let body = "Closes #510\n\
                    AC3: covered by `crates/a/tests/campaign.rs::lists_entities` — the list exists\n\
                    - AC4, AC5: Covered by tests/rules/R103.spec.ts — the rule runs it\n\
                    AC6: covered by tests/x.rs\n\
                    AC9: covered by tests/z.rs —\n\
                    AC7: covered by — nothing\n\
                    covered by tests/y.rs — no criterion\n\
                    AC8: covered by ``:: — empty";
        assert_eq!(
            declared_covered(body),
            [
                Pointer {
                    criteria: BTreeSet::from([3]),
                    path: "crates/a/tests/campaign.rs".to_string(),
                    test: Some("lists_entities".to_string()),
                },
                Pointer {
                    criteria: BTreeSet::from([4, 5]),
                    path: "tests/rules/R103.spec.ts".to_string(),
                    test: None,
                },
            ]
        );
    }

    #[test]
    fn a_pointer_proves_only_a_test_file_holding_its_test() {
        let pointer = |path: &str, test: Option<&str>| Pointer {
            criteria: BTreeSet::from([1]),
            path: path.to_string(),
            test: test.map(ToString::to_string),
        };
        assert!(proves(&pointer("tests/a.rs", None), "fn x() {}"));
        assert!(proves(
            &pointer("tests/a.rs", Some("lists")),
            "fn lists() {}"
        ));
        assert!(!proves(
            &pointer("tests/a.rs", Some("gone")),
            "fn lists() {}"
        ));
        assert!(!proves(&pointer("src/lib.rs", None), "fn shipped() {}"));
    }

    proptest! {
        #[test]
        fn bullets_written_are_read_back_whole(
            items in proptest::collection::vec("[a-zA-Z][a-zA-Z ,]{0,30}", 1..8)
        ) {
            let section = items
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n");
            let expected: Vec<String> = items.iter().map(|item| item.trim().to_string()).collect();
            let found = criteria(&section);
            prop_assert_eq!(&found, &expected);
            prop_assert_eq!(criteria(&numbered(&found)), expected);
        }

        #[test]
        fn any_section_parses_into_at_most_one_criterion_per_line(text in "(\\PC|\n){0,200}") {
            let found = criteria(&text);
            prop_assert!(found.len() <= text.lines().count());
            prop_assert!(found.iter().all(|item| !item.trim().is_empty()));
        }

        #[test]
        fn every_number_written_as_a_word_is_named(numbers in proptest::collection::vec(1usize..500, 0..6)) {
            let text = numbers
                .iter()
                .map(|n| format!("// AC{n}: x"))
                .collect::<Vec<_>>()
                .join("\n");
            prop_assert_eq!(named(&text), numbers.into_iter().collect::<BTreeSet<_>>());
        }

        #[test]
        fn a_uuid_names_no_criterion(
            uuid in "[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}",
            forced in "ac[0-9]{6}-ac[0-9]{2}-4[0-9a-f]{3}-ac[0-9]{2}-ac[0-9]{10}",
        ) {
            for id in [&uuid, &forced] {
                let fixture = format!("let id = \"{id}\";\nconst UP: &str = \"{}\";", id.to_uppercase());
                prop_assert!(named(&fixture).is_empty(), "{}", fixture);
            }
        }
    }
}
