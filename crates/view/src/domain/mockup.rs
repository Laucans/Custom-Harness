//! The product's mock-up as the easel shows it: which branch a path names,
//! what `mock/pages.json` says, which files may be served, and the branch a
//! saved mock-up goes to. Pure: the `Mockups` port does the reading.
//!
//! A branch travels in a URL as a **slug** — `milestone/3-foo` is
//! `milestone__3-foo` — so a page's relative links (`../style.css`,
//! `fixtures/x.json`) resolve under `/mock/<slug>/`. The slug is the
//! convention the demo's folders already use.

use crate::ports::MockPage;

/// The longest branch name accepted from a URL.
const MAX_BRANCH: usize = 200;

/// The longest file path accepted from a URL.
const MAX_PATH: usize = 500;

/// A branch as a URL segment: `/` becomes `__`.
#[must_use]
pub fn slug(branch: &str) -> String {
    branch.replace('/', "__")
}

/// The branch a URL segment names: `__` becomes `/`. `None` for a segment
/// no branch could have.
#[must_use]
pub fn branch_of(slug: &str) -> Option<String> {
    let branch = slug.replace("__", "/");
    let plain = !branch.is_empty()
        && branch.len() <= MAX_BRANCH
        && !branch.starts_with('/')
        && !branch.ends_with('/')
        && !branch.contains("..")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c));
    plain.then_some(branch)
}

/// A file path under `mock/` safe to serve: relative, no `..`, no
/// backslash, no empty segment, at most [`MAX_PATH`] characters.
#[must_use]
pub fn is_safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// The pages of a `mock/pages.json`.
///
/// # Errors
///
/// The text is not the manifest contract N describes.
pub fn pages_of(json: &str) -> Result<Vec<MockPage>, String> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        #[serde(default)]
        pages: Vec<MockPage>,
    }
    serde_json::from_str::<Manifest>(json)
        .map(|m| m.pages)
        .map_err(|e| format!("mock/pages.json: {e}"))
}

/// The media type a file is served with, by its extension.
#[must_use]
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The branch a saved mock-up goes to: `mockup/<name>`, the name made plain
/// (lower case, digits, dashes), `mockup/pages` when nothing is left of it.
#[must_use]
pub fn save_branch(name: &str) -> String {
    let mut plain = String::new();
    let mut dash = false;
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            plain.push(c);
            dash = false;
        } else if !dash && !plain.is_empty() {
            plain.push('-');
            dash = true;
        }
    }
    let plain = plain.trim_end_matches('-');
    let plain: String = plain.chars().take(60).collect();
    if plain.is_empty() {
        "mockup/pages".to_string()
    } else {
        format!("mockup/{plain}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_branch_travels_as_a_slug_and_comes_back() {
        assert_eq!(slug("milestone/3-foo"), "milestone__3-foo");
        assert_eq!(
            branch_of("milestone__3-foo").as_deref(),
            Some("milestone/3-foo")
        );
        assert_eq!(branch_of("main_agent").as_deref(), Some("main_agent"));
        assert!(branch_of("").is_none());
        assert!(branch_of("a__..__b").is_none());
        assert!(branch_of("__a").is_none());
        assert!(branch_of("a b").is_none());
    }

    #[test]
    fn only_a_plain_relative_path_is_served() {
        assert!(is_safe_path("pages/home.html"));
        assert!(is_safe_path("fixtures/x.json"));
        assert!(!is_safe_path(""));
        assert!(!is_safe_path("/etc/passwd"));
        assert!(!is_safe_path("../secret"));
        assert!(!is_safe_path("pages//x.html"));
        assert!(!is_safe_path("a\\b"));
    }

    #[test]
    fn the_manifest_lists_its_pages() {
        let pages = pages_of(
            r#"{"version":1,"pages":[{"path":"pages/a.html","title":"A","objective":"o","screen":"a"}]}"#,
        )
        .expect("a manifest");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].screen, "a");
        assert!(pages_of("nope").is_err());
        assert!(pages_of("{}").expect("empty").is_empty());
    }

    #[test]
    fn a_saved_mockup_gets_a_plain_branch_and_a_page_a_plain_file() {
        assert_eq!(save_branch("Credit review!"), "mockup/credit-review");
        assert_eq!(save_branch("  "), "mockup/pages");
        assert_eq!(content_type("pages/a.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("x.bin"), "application/octet-stream");
    }

    proptest! {
        #[test]
        fn a_save_branch_is_always_a_plain_git_name(name in "\\PC{0,80}") {
            let branch = save_branch(&name);
            prop_assert!(branch.starts_with("mockup/"));
            prop_assert!(branch.len() <= 67);
            prop_assert!(branch["mockup/".len()..].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
            prop_assert!(!branch.ends_with('-'));
        }

        #[test]
        fn a_slug_round_trips_any_plain_branch(branch in "[a-z0-9][a-z0-9-]{0,10}(/[a-z0-9][a-z0-9-]{0,10}){0,2}") {
            prop_assert_eq!(branch_of(&slug(&branch)), Some(branch));
        }
    }
}
