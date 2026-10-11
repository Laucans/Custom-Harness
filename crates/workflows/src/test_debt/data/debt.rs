//! What a test-debt issue says: the file it is about, and what was found.
//!
//! The issue is written by the CI (`scripts/test-debt.sh`, installed by
//! `init-repo`): titled `Test debt: <file>`, its body the mutants no test
//! caught, each later finding a comment. Pure: reads texts already fetched.

/// What every test-debt issue's title starts with, before the file.
pub const TITLE_PREFIX: &str = "Test debt: ";

/// The source file a test-debt title is about, if it names one.
///
/// Only a relative Rust source path, written with the characters a repository
/// path holds: the file is spliced into a prompt and into the commands the
/// session runs, so a title a human mistyped — or wrote to be clever — names
/// nothing rather than something else.
#[must_use]
pub fn file_of(title: &str) -> Option<String> {
    let file = title.strip_prefix(TITLE_PREFIX)?.trim();
    let safe = file
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'));
    let relative = !file.starts_with('/') && !file.split('/').any(|part| part == "..");
    // `.rs` alone is a hidden file with no extension, not a source.
    let rust = std::path::Path::new(file)
        .extension()
        .is_some_and(|ext| ext == "rs");
    (safe && relative && rust).then(|| file.to_string())
}

/// What the mutation runs found, oldest first: the issue's body, then each
/// comment — a later run's comment says what is missed now.
#[must_use]
pub fn findings(body: &str, comments: &[String]) -> String {
    std::iter::once(body)
        .chain(comments.iter().map(String::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<&str>>()
        .join("\n\n---\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_title_the_ci_writes_names_its_file() {
        assert_eq!(
            file_of("Test debt: crates/notify/src/rules.rs").as_deref(),
            Some("crates/notify/src/rules.rs")
        );
        // The script that opens the issue writes this very prefix.
        let script =
            harness_architecture::assets::content_of("scripts/test-debt.sh").unwrap_or_default();
        assert!(script.contains("title=\"Test debt: $file\""), "{script}");
    }

    #[test]
    fn a_title_naming_anything_but_a_relative_rust_file_names_nothing() {
        for title in [
            "Test debt: ",
            "Test debt: .rs",
            "Test debt: README.md",
            "Test debt: /etc/passwd.rs",
            "Test debt: ../outside/src/lib.rs",
            "Test debt: src/lib.rs; rm -rf .",
            "Test debt: src/$(whoami).rs",
            "test debt: src/lib.rs",
            "Bug: src/lib.rs",
        ] {
            assert_eq!(file_of(title), None, "{title}");
        }
    }

    #[test]
    fn the_findings_read_oldest_first_and_skip_what_is_empty() {
        let said = findings(
            "- line 3: replace f with true\n",
            &["  ".to_string(), "Still missed in run 9".to_string()],
        );
        assert_eq!(
            said,
            "- line 3: replace f with true\n\n---\n\nStill missed in run 9"
        );
        assert_eq!(findings("", &[]), "");
    }

    proptest! {
        #[test]
        fn any_relative_source_path_comes_back_from_its_title(
            path in "[a-z_]{1,8}(/[a-z0-9_-]{1,8}){0,4}\\.rs"
        ) {
            prop_assert_eq!(file_of(&format!("{TITLE_PREFIX}{path}")), Some(path));
        }

        #[test]
        fn no_title_makes_it_panic(title in ".*") {
            let _ = file_of(&title);
        }
    }
}
