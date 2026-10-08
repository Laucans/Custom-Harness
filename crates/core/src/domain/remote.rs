//! A GitHub repository named by a URL or `owner/name`, and whether two such
//! names agree. Pure, no I/O — `github.com` is this module's entire world,
//! and it names no other host.

/// `owner/name`, as `gh api repos/{slug}/…` wants it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Slug {
    /// The repository's owner — a user or an organization.
    pub owner: String,
    /// The repository's name.
    pub name: String,
}

impl std::fmt::Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl Slug {
    /// Parses `https://github.com/o/r[.git]`, `git@github.com:o/r.git`,
    /// `ssh://git@github.com/o/r.git`, or the `o/r` shorthand.
    ///
    /// Refuses anything that is not `github.com`, and refuses an owner or
    /// name outside `[A-Za-z0-9._-]+` — including `.` and `..` — **before it
    /// reaches a `gh api` path**: the URL is human input interpolated into
    /// `repos/{slug}/…`, and `..` there is path traversal on someone else's
    /// repository.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let tail = if let Some(rest) = text.strip_prefix("https://github.com/") {
            rest
        } else if let Some(rest) = text.strip_prefix("http://github.com/") {
            rest
        } else if let Some(rest) = text.strip_prefix("ssh://git@github.com/") {
            rest
        } else if let Some(rest) = text.strip_prefix("git@github.com:") {
            rest
        } else if text.contains("://") || text.contains('@') || text.contains(':') {
            // A scheme, an `@`, or a `:` that didn't match a github.com
            // prefix above: another host or another scheme.
            return None;
        } else {
            // No scheme, no `@`, no `:`: only the `o/r` shorthand is this
            // shape.
            text
        };

        let tail = tail.trim_end_matches('/');
        let tail = tail.strip_suffix(".git").unwrap_or(tail);
        let (owner, name) = tail.split_once('/')?;
        if name.is_empty() || name.contains('/') {
            return None;
        }
        if !is_segment(owner) || !is_segment(name) {
            return None;
        }
        Some(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }
}

/// One `owner` or `name` segment: non-empty, `[A-Za-z0-9._-]+`, and not `.`
/// or `..` — refused before it reaches a `gh api` path.
fn is_segment(text: &str) -> bool {
    !text.is_empty()
        && text != "."
        && text != ".."
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Two URLs that name the same repo, writing form aside.
///
/// `git@github.com:o/r.git` and `https://github.com/o/r` are the same repo;
/// comparing them character by character would reject a perfectly valid
/// workspace. Moved from `execution::provisioning` — one spelling of "these
/// two URLs are the same repo", not two.
#[must_use]
pub fn same_repo(one: &str, other: &str) -> bool {
    key(one) == key(other)
}

fn key(url: &str) -> String {
    let mut text = url.trim_end_matches('/');
    text = text.strip_suffix(".git").unwrap_or(text);
    for prefix in ["https://", "http://", "ssh://", "git://"] {
        text = text.strip_prefix(prefix).unwrap_or(text);
    }
    let text = text.replace(':', "/");
    let text = text.strip_prefix("git@").unwrap_or(&text);
    // The `owner/name` shorthand names the same repository as its
    // github.com URL: `init-repo` writes the shorthand into `.env.local`,
    // and a clone's `origin` is always the URL.
    text.strip_prefix("github.com/")
        .unwrap_or(text)
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_shorthand_and_the_github_url_are_the_same_repository() {
        assert!(same_repo(
            "Laucans/dnd_helper",
            "https://github.com/Laucans/dnd_helper"
        ));
        assert!(same_repo(
            "git@github.com:Laucans/dnd_helper.git",
            "laucans/dnd_helper"
        ));
        assert!(!same_repo("Laucans/dnd_helper", "Laucans/other"));
    }

    #[test]
    fn an_https_url_a_git_ssh_url_and_the_shorthand_all_yield_the_same_slug() {
        let want = Slug {
            owner: "Laucans".to_string(),
            name: "event_assistant".to_string(),
        };
        assert_eq!(
            Slug::parse("https://github.com/Laucans/event_assistant"),
            Some(want.clone())
        );
        assert_eq!(
            Slug::parse("https://github.com/Laucans/event_assistant.git"),
            Some(want.clone())
        );
        assert_eq!(
            Slug::parse("git@github.com:Laucans/event_assistant.git"),
            Some(want.clone())
        );
        assert_eq!(
            Slug::parse("ssh://git@github.com/Laucans/event_assistant.git"),
            Some(want.clone())
        );
        assert_eq!(Slug::parse("Laucans/event_assistant"), Some(want));
    }

    #[test]
    fn the_display_form_is_owner_slash_name() {
        let slug = Slug::parse("o/r").expect("parses");
        assert_eq!(slug.to_string(), "o/r");
    }

    #[test]
    fn a_non_github_host_is_refused() {
        assert_eq!(Slug::parse("https://gitlab.com/o/r"), None);
        assert_eq!(Slug::parse("https://www.github.com/o/r"), None);
    }

    #[test]
    fn dot_dot_in_owner_or_name_is_refused_before_it_reaches_an_api_path() {
        assert_eq!(Slug::parse("https://github.com/../r"), None);
        assert_eq!(Slug::parse("https://github.com/o/.."), None);
        assert_eq!(Slug::parse("../.."), None);
    }

    #[test]
    fn a_malformed_shorthand_is_refused_not_guessed() {
        assert_eq!(Slug::parse("just-a-name"), None);
        assert_eq!(Slug::parse(""), None);
        assert_eq!(Slug::parse("o/"), None);
        assert_eq!(Slug::parse("o//r"), None);
    }

    proptest! {
        #[test]
        fn arbitrary_text_never_panics_and_never_yields_an_unsafe_segment(text in ".*") {
            if let Some(slug) = Slug::parse(&text) {
                let safe = |s: &str| {
                    !s.is_empty()
                        && s != "."
                        && s != ".."
                        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                };
                prop_assert!(safe(&slug.owner));
                prop_assert!(safe(&slug.name));
            }
        }
    }

    #[test]
    fn two_spellings_of_the_same_remote_are_the_same_repository() {
        assert!(same_repo(
            "git@github.com:Laucans/event_assistant.git",
            "https://github.com/Laucans/event_assistant"
        ));
        assert!(same_repo(
            "https://github.com/o/r/",
            "https://github.com/o/r"
        ));
    }

    #[test]
    fn two_repositories_with_the_same_tail_are_not_the_same_remote() {
        assert!(!same_repo(
            "git@github.com:someone/event_assistant.git",
            "git@github.com:Laucans/event_assistant.git"
        ));
    }
}
