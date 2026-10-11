//! Moving the view from one factory to another: the command line it starts
//! again with. No process here, only the words.

use std::ffi::OsString;

use harness_core::ports::store::factories::Factory;

/// The two options the factory owns: what the previous factory had on the
/// command line must not outvote them.
const OWNED: [&str; 2] = ["--target-repo-url", "--integration-branch"];

/// The arguments the view starts again with in `factory`: the ones it was
/// started with, minus the repository and branch of the factory it leaves,
/// plus those of the one it enters.
#[must_use]
pub fn args_for(original: &[OsString], factory: &Factory) -> Vec<OsString> {
    let mut args = Vec::with_capacity(original.len() + 4);
    let mut value_follows = false;
    for arg in original {
        if std::mem::take(&mut value_follows) {
            continue;
        }
        match arg.to_str() {
            Some(text) if OWNED.contains(&text) => value_follows = true,
            Some(text)
                if OWNED
                    .iter()
                    .any(|flag| text.strip_prefix(flag).is_some_and(|r| r.starts_with('='))) => {}
            _ => args.push(arg.clone()),
        }
    }
    args.extend([
        OsString::from("--target-repo-url"),
        OsString::from(&factory.slug),
        OsString::from("--integration-branch"),
        OsString::from(&factory.integration_branch),
    ]);
    args
}

/// The variables a factory's `.env.local` gives the view it starts again
/// in: `KEY=VALUE` lines, comments and blanks skipped, quotes taken off.
/// An empty value means "the default" and is left out, and so is the port:
/// the page waits for the view on the port it already talks to.
#[must_use]
pub fn env_of(text: &str) -> Vec<(String, String)> {
    // A blank line has no `=`, and a comment's key has a `#`: neither is a variable.
    text.lines()
        .map(str::trim)
        .filter_map(|line| {
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            let usable = !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !value.is_empty()
                && key != "HARNESS_VIEW_PORT";
            usable.then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_factorys_env_file_gives_its_set_values_but_never_the_port() {
        let text = "# a comment\n#COMMENTED=x\n# SPACED = y\n\nTARGET_REPO_URL=me/beta\nSTAGES=\nexport HARNESS_BIN=\"/h/harness\"\n\
                    HARNESS_WATCH_COMMAND='/h/harness watch --force-reset'\nHARNESS_VIEW_PORT=7879\n\
                    not a line\nBAD KEY=x\nX=a=b\n";
        assert_eq!(
            env_of(text),
            [
                ("TARGET_REPO_URL", "me/beta"),
                ("HARNESS_BIN", "/h/harness"),
                ("HARNESS_WATCH_COMMAND", "/h/harness watch --force-reset"),
                ("X", "a=b"),
            ]
            .map(|(k, v)| (k.to_string(), v.to_string()))
        );
    }

    fn factory() -> Factory {
        Factory {
            id: "beta".to_string(),
            name: "beta".to_string(),
            slug: "me/beta".to_string(),
            root: "/work/beta".into(),
            integration_branch: "main".to_string(),
        }
    }

    fn words(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_new_factorys_repository_and_branch_close_the_line() {
        assert_eq!(
            args_for(&words(&["--port", "7000"]), &factory()),
            words(&[
                "--port",
                "7000",
                "--target-repo-url",
                "me/beta",
                "--integration-branch",
                "main"
            ])
        );
    }

    #[test]
    fn the_old_factorys_repository_and_branch_are_dropped_in_both_spellings() {
        let original = words(&[
            "--target-repo-url",
            "me/alpha",
            "--demo",
            "--integration-branch=main_agent",
            "--port=7000",
        ]);
        assert_eq!(
            args_for(&original, &factory()),
            words(&[
                "--demo",
                "--port=7000",
                "--target-repo-url",
                "me/beta",
                "--integration-branch",
                "main"
            ])
        );
    }

    #[test]
    fn an_option_that_only_starts_with_an_owned_name_stays() {
        let original = words(&["--target-repo-urls", "x"]);
        assert_eq!(
            args_for(&original, &factory())[..2],
            words(&["--target-repo-urls", "x"])[..]
        );
    }

    #[test]
    fn an_unlinked_factory_leaves_the_repository_empty() {
        let unlinked = Factory {
            slug: String::new(),
            ..factory()
        };
        assert_eq!(
            args_for(&[], &unlinked)[..2],
            words(&["--target-repo-url", ""])[..]
        );
    }
}
