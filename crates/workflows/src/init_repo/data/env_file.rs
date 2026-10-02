//! The in-place rewrite of `.env.local`: "this text, with these keys set".
//!
//! Pure `String -> String`, which is what makes the dangerous part —
//! touching the human's `.env.local` — testable without a filesystem.

/// Sets `TARGET_REPO_URL` and `INTEGRATION_BRANCH` in this text.
///
/// A `KEY=` line is updated in place, appended otherwise. Every other line,
/// comment or blank — and the order of what already existed — survives
/// untouched.
#[must_use]
pub fn set_keys(text: &str, target_repo_url: &str, integration_branch: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    set_key(&mut lines, "TARGET_REPO_URL", target_repo_url);
    set_key(&mut lines, "INTEGRATION_BRANCH", integration_branch);
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn set_key(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    if let Some(line) = lines.iter_mut().find(|line| line.starts_with(&prefix)) {
        *line = format!("{key}={value}");
    } else {
        lines.push(format!("{key}={value}"));
    }
}

/// The value of `TARGET_REPO_URL` already in this text, if the key exists
/// and the line is non-blank — used to detect the `--force`-guarded
/// conflict.
#[must_use]
pub fn existing_target_repo_url(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("TARGET_REPO_URL="))
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_updated_in_place() {
        let text = "FOO=bar\nTARGET_REPO_URL=old\nBAZ=qux\n";
        let updated = set_keys(text, "o/r", "main_agent");
        assert_eq!(
            updated,
            "FOO=bar\nTARGET_REPO_URL=o/r\nBAZ=qux\nINTEGRATION_BRANCH=main_agent\n"
        );
    }

    #[test]
    fn comments_and_unrelated_lines_survive_byte_for_byte() {
        let text = "# a comment\nFOO=bar\n\n# another\nBAZ=qux\n";
        let updated = set_keys(text, "o/r", "main_agent");
        assert!(updated.contains("# a comment\nFOO=bar\n\n# another\nBAZ=qux\n"));
    }

    #[test]
    fn a_missing_key_is_appended_not_inserted_mid_file() {
        let text = "FOO=bar\n";
        let updated = set_keys(text, "o/r", "main_agent");
        assert_eq!(
            updated,
            "FOO=bar\nTARGET_REPO_URL=o/r\nINTEGRATION_BRANCH=main_agent\n"
        );
    }

    #[test]
    fn an_absent_key_has_no_existing_url() {
        assert_eq!(existing_target_repo_url("FOO=bar\n"), None);
    }

    #[test]
    fn a_blank_value_is_not_an_existing_url() {
        assert_eq!(existing_target_repo_url("TARGET_REPO_URL=\n"), None);
    }

    #[test]
    fn a_set_value_is_read_back() {
        assert_eq!(
            existing_target_repo_url("TARGET_REPO_URL=o/r\n"),
            Some("o/r".to_string())
        );
    }
}
