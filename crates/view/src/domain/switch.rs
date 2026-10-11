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

#[cfg(test)]
mod tests {
    use super::*;

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
