//! The product's mock-up as the easel shows it: which branch a path names,
//! what `mock/pages.json` says, which files may be served, and the branch a
//! saved mock-up goes to. Pure: the `Mockups` port does the reading.
//!
//! A branch travels in a URL as a **slug** — `milestone/3-foo` is
//! `milestone__3-foo` — so a page's relative links (`../style.css`,
//! `fixtures/x.json`) resolve under `/mock/<slug>/`. The slug is the
//! convention the demo's folders already use.

use crate::ports::{MockComponent, MockPage, UnitFile};

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

/// What a `mock/pages.json` lists: its pages and its micro-frontends.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    /// The pages.
    #[serde(default)]
    pub pages: Vec<MockPage>,
    /// The micro-frontends.
    #[serde(default)]
    pub components: Vec<MockComponent>,
}

/// The pages and micro-frontends of a `mock/pages.json`.
///
/// # Errors
///
/// The text is not the manifest contract N describes.
pub fn manifest_of(json: &str) -> Result<Manifest, String> {
    serde_json::from_str::<Manifest>(json).map_err(|e| format!("mock/pages.json: {e}"))
}

/// The shape of `mock/pages.json`, one page and one micro-frontend of
/// it, written from the very types [`manifest_of`] reads — what a session
/// is shown so it never guesses the contract.
#[must_use]
pub fn manifest_shape() -> String {
    let props = [("status".to_string(), "\"open\" | \"paid\"".to_string())];
    let sample = Manifest {
        pages: vec![MockPage {
            path: "pages/invoices.html".to_string(),
            title: "Invoices".to_string(),
            objective: "The accountant finds an unpaid invoice and marks it paid".to_string(),
            screen: String::new(),
            components: vec!["InvoiceList".to_string()],
            hash: String::new(),
        }],
        components: vec![MockComponent {
            name: "InvoiceList".to_string(),
            system: "billing".to_string(),
            description: "The invoices, filtered by status".to_string(),
            fragment: "components/InvoiceList.html".to_string(),
            fixtures: vec!["fixtures/billing.listInvoices.json".to_string()],
            needs: vec!["billing.listInvoices".to_string()],
            actions: vec!["billing.markInvoicePaid".to_string()],
            props: props.into_iter().collect(),
            ..MockComponent::default()
        }],
    };
    serde_json::to_string_pretty(&sample).unwrap_or_default()
}

/// What the manifests of a branch say that the micro-frontend designer
/// offers: the Micro-UIs built, and the Capabilities and `DataCapabilities`
/// a new one may need or trigger.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Units {
    /// The Micro-UIs declared under `apps/`, as mock components would be.
    pub micro_uis: Vec<MockComponent>,
    /// Every Capability id, `credit.calculateRisk`.
    pub capabilities: Vec<String>,
    /// Every `DataCapability` id.
    pub data_capabilities: Vec<String>,
}

/// Reads the manifests of a branch.
#[must_use]
pub fn units_of(files: &[UnitFile]) -> Units {
    let mut units = Units::default();
    for file in files {
        let name = file.path.rsplit('/').next().unwrap_or_default();
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&file.text) else {
            continue;
        };
        let text = |key: &str| json.get(key).and_then(|v| v.as_str()).map(str::to_string);
        let list = |key: &str| -> Vec<String> {
            json.get(key)
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| {
                            v.as_str().map(str::to_string).or_else(|| {
                                v.get("capability")
                                    .or_else(|| v.get("dataCapability"))
                                    .or_else(|| v.get("id"))
                                    .and_then(|x| x.as_str())
                                    .map(str::to_string)
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        match name {
            "capability.json" => units.capabilities.extend(text("capability")),
            "data-capability.json" => units.data_capabilities.extend(text("dataCapability")),
            "micro-ui.json" => {
                if let Some(name) = text("microUi") {
                    units.micro_uis.push(MockComponent {
                        name,
                        system: text("system").unwrap_or_default(),
                        description: text("description").unwrap_or_default(),
                        fragment: String::new(),
                        fixtures: Vec::new(),
                        needs: list("needs"),
                        actions: list("actions"),
                        source: String::new(),
                        built_from: String::new(),
                        props: json
                            .get("props")
                            .and_then(|v| v.as_object())
                            .map(|props| {
                                props
                                    .iter()
                                    .map(|(k, v)| {
                                        (k.clone(), v.as_str().unwrap_or_default().to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    });
                }
            }
            _ => {}
        }
    }
    units.capabilities.sort();
    units.capabilities.dedup();
    units.data_capabilities.sort();
    units.data_capabilities.dedup();
    units.micro_uis.sort_by(|a, b| a.name.cmp(&b.name));
    units
}

/// The page that previews one micro-frontend alone: the mock-up's style
/// and loader, and one `data-component` element — served at
/// `/mock/<slug>/__preview/<Name>` beside the pages.
#[must_use]
pub fn preview_page(name: &str) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{name}</title>\
         <link rel=\"stylesheet\" href=\"../design/tokens.css\">\
         <link rel=\"stylesheet\" href=\"../kit/kit.css\">\
         <link rel=\"stylesheet\" href=\"../style.css\"></head>\n\
         <body style=\"margin:0;padding:16px;background:transparent\">\n\
         <div data-component=\"{name}\"></div>\n\
         <script src=\"../mock.js\"></script>\n</body></html>\n"
    )
}

/// The micro-frontends a page's markup injects, by name, in order, once
/// each: every `data-component="<Name>"`, quoted either way.
#[must_use]
pub fn injected(html: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (i, _) in html.match_indices("data-component=") {
        let rest = html.get(i + "data-component=".len()..).unwrap_or_default();
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let name: String = rest.chars().skip(1).take_while(|c| *c != quote).collect();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// What a file of the mock-up is, by where it lies under `mock/`: what
/// the page counts while a session draws. A micro-frontend's draft is a
/// proposal, not a micro-frontend.
#[must_use]
pub fn kind_of(path: &str) -> &'static str {
    match path.split('/').next().unwrap_or_default() {
        "components" if path.ends_with(".draft.html") => "draft",
        "pages" => "page",
        "components" => "micro-frontend",
        "fixtures" => "fixture",
        "kit" => "kit",
        "design" => "design",
        _ if path == "pages.json" => "manifest",
        _ => "other",
    }
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

    #[test]
    fn a_page_s_micro_frontends_are_read_off_its_markup() {
        let html = r#"<div data-component="Rail"></div><p data-component='Sheet'></p><i data-component="Rail"></i><b data-component=></b>"#;
        assert_eq!(injected(html), ["Rail", "Sheet"]);
        let seen = &injected("<main></main>");
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn a_file_s_kind_is_where_it_lies() {
        for (path, kind) in [
            ("pages/home.html", "page"),
            ("components/List.html", "micro-frontend"),
            ("components/List.draft.html", "draft"),
            ("fixtures/media/a.svg", "fixture"),
            ("kit/Button.html", "kit"),
            ("design/tokens.css", "design"),
            ("pages.json", "manifest"),
            ("mock.js", "other"),
        ] {
            assert_eq!(kind_of(path), kind, "{path}");
        }
    }
    use proptest::prelude::*;

    #[test]
    fn the_manifest_shape_shown_to_a_session_reads_back() {
        let shape = manifest_shape();
        let read = manifest_of(&shape).expect("the shape is the contract");
        assert_eq!((read.pages.len(), read.components.len()), (1, 1));
        let c = &read.components[0];
        assert_eq!(
            c.fixtures,
            vec!["fixtures/billing.listInvoices.json".to_string()]
        );
        assert_eq!(c.props.len(), 1);
        assert!(
            shape.contains("\"fixtures\": [\n"),
            "the fixtures are always a list, one per Capability: {shape}"
        );
        assert!(
            !shape.contains("\"hash\""),
            "what a session never writes is not shown"
        );
    }

    #[test]
    fn fixtures_are_a_list_and_an_older_single_one_is_still_read() {
        let read = |key: &str, fixture: &str| {
            manifest_of(&format!(
                r#"{{"pages":[],"components":[{{"name":"Map","{key}":{fixture}}}]}}"#
            ))
            .map(|m| m.components[0].fixtures.clone())
        };
        let files = |f: &[&str]| f.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            read("fixture", r#""fixtures/a.json""#),
            Ok(files(&["fixtures/a.json"]))
        );
        assert_eq!(
            read("fixtures", r#"["fixtures/a.b.json","fixtures/c.d.json"]"#),
            Ok(files(&["fixtures/a.b.json", "fixtures/c.d.json"]))
        );
        assert_eq!(
            read("fixture", r#""""#),
            Ok(Vec::new()),
            "none written: none"
        );
        assert!(read("fixtures", "3").is_err(), "neither a file nor a list");
        let one = MockComponent {
            fixtures: files(&["fixtures/a.b.json"]),
            ..MockComponent::default()
        };
        let written = serde_json::to_value(&one).expect("json");
        assert_eq!(
            written["fixtures"],
            serde_json::json!(["fixtures/a.b.json"])
        );
        assert!(
            written.get("fixture").is_none(),
            "the old key is never written"
        );
        let none = serde_json::to_value(MockComponent::default()).expect("json");
        assert!(none.get("fixtures").is_none(), "no fixture, no field");
    }

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
        let pages = manifest_of(
            r#"{"version":1,"pages":[{"path":"pages/a.html","title":"A","objective":"o","screen":"a"}]}"#,
        )
        .expect("a manifest")
        .pages;
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].screen, "a");
        assert!(manifest_of("nope").is_err());
        assert!(
            manifest_of("{}").expect("empty").pages.is_empty(),
            "expected nothing"
        );
    }

    #[test]
    fn the_manifests_say_what_the_designer_offers() {
        let files = vec![
            UnitFile {
                path: "apps/credit/risk-badge/micro-ui.json".into(),
                text: r#"{"microUi":"RiskBadge","system":"credit","description":"d","needs":["credit.calculateRisk"],"actions":[{"dataCapability":"credit.requestLimitChange"}],"props":{"customerId":"ID"}}"#.into(),
            },
            UnitFile {
                path: "crates/credit/capabilities/calculate-risk/capability.json".into(),
                text: r#"{"capability":"credit.calculateRisk"}"#.into(),
            },
            UnitFile {
                path: "crates/dataguard/data-capabilities/x/data-capability.json".into(),
                text: r#"{"dataCapability":"credit.requestLimitChange"}"#.into(),
            },
            UnitFile {
                path: "apps/x/micro-ui.json".into(),
                text: "nope".into(),
            },
        ];
        let units = units_of(&files);
        assert_eq!(units.capabilities, ["credit.calculateRisk"]);
        assert_eq!(units.data_capabilities, ["credit.requestLimitChange"]);
        assert_eq!(units.micro_uis.len(), 1);
        assert_eq!(units.micro_uis[0].actions, ["credit.requestLimitChange"]);
        assert_eq!(units.micro_uis[0].props["customerId"], "ID");
        let manifest = manifest_of(
            r#"{"version":1,"pages":[],"components":[{"name":"RiskBadge","system":"credit","description":"d","fragment":"components/RiskBadge.html"}]}"#,
        )
        .expect("a manifest");
        assert_eq!(manifest.components[0].fragment, "components/RiskBadge.html");
        let preview = preview_page("RiskBadge");
        assert!(preview.contains(r#"data-component="RiskBadge""#));
        // Styled as a page is: the design system's variables and kit first.
        let tokens = preview.find("../design/tokens.css").expect("tokens");
        let kit = preview.find("../kit/kit.css").expect("kit");
        let own = preview.find("../style.css").expect("style");
        assert!(tokens < kit && kit < own, "{preview}");
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
