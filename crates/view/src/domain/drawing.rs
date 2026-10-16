//! The proof of concept drawn screen by screen: the screens read off the
//! scope, the page each one gets, the manifest the desk keeps itself —
//! sessions drawing in parallel never write it — and the questions put to
//! the sessions that draw the reference page and the others after it.
//!
//! Pure: texts in, texts out.

use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::domain::creation::{Creation, design_brief, product};

/// The headings the scope's screens may sit under: the one the scope
/// prompt asks for, and the French a brief written in French may bring.
const SCREENS_HEADINGS: [&str; 3] = ["screens", "écrans", "ecrans"];

/// One screen of the proof of concept, as the scope lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Screen {
    /// Its name, as the scope writes it in bold.
    pub name: String,
    /// What the user sees and does there.
    pub description: String,
}

/// The screens of a scope's `## Screens` section, in order, each once.
#[must_use]
pub fn screens(scope: &str) -> Vec<Screen> {
    let mut found: Vec<Screen> = Vec::new();
    let mut inside = false;
    for line in scope.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            let title = heading.trim_start_matches('#').trim().to_lowercase();
            inside = SCREENS_HEADINGS.iter().any(|h| title.starts_with(h));
            continue;
        }
        if !inside || trimmed.is_empty() {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let Some(item) = bullet(trimmed) else {
            // A paragraph under a bullet goes with it.
            if let Some(last) = found.last_mut() {
                join(&mut last.description, trimmed);
            }
            continue;
        };
        if indented && let Some(last) = found.last_mut() {
            join(&mut last.description, item);
            continue;
        }
        let (name, description) = split_item(item);
        if name.is_empty()
            || found
                .iter()
                .any(|s| s.name.to_lowercase() == name.to_lowercase())
        {
            continue;
        }
        found.push(Screen { name, description });
    }
    found
}

/// The text of a list item, its marker taken off; `None` when the line is
/// not one.
fn bullet(line: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = line.get(digits..)?;
        return rest
            .strip_prefix(". ")
            .or_else(|| rest.strip_prefix(") "))
            .map(str::trim);
    }
    None
}

/// A screen's name and description out of its list item: the name in bold
/// when there is one, else what comes before the first separator.
fn split_item(item: &str) -> (String, String) {
    if let Some(open) = item.find("**")
        && let Some(close) = item.get(open + 2..).and_then(|rest| rest.find("**"))
    {
        let name = item.get(open + 2..open + 2 + close).unwrap_or_default();
        let rest = item.get(open + 4 + close..).unwrap_or_default();
        return (clean_name(name), clean_description(rest));
    }
    for separator in [" — ", " – ", " - ", ": "] {
        if let Some((name, rest)) = item.split_once(separator) {
            return (clean_name(name), clean_description(rest));
        }
    }
    (clean_name(item), String::new())
}

fn clean_name(name: &str) -> String {
    name.trim().trim_end_matches([':', '.']).trim().to_string()
}

fn clean_description(rest: &str) -> String {
    rest.trim()
        .trim_start_matches([':', '—', '–', '-', '.'])
        .trim()
        .to_string()
}

fn join(text: &mut String, more: &str) {
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(more);
}

/// A screen's name folded into a file name: lowercase ASCII letters and
/// digits, dashes between words, the usual accents taken off.
#[must_use]
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        let folded = match c {
            'à' | 'â' | 'ä' | 'á' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        };
        if folded.is_ascii_alphanumeric() {
            out.push(folded);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "screen".to_string()
    } else {
        out
    }
}

/// One screen and the page it is drawn on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Planned {
    /// The screen.
    pub screen: Screen,
    /// Its page, relative to `mock/`: `pages/invoices.html`.
    pub path: String,
}

/// Every screen with its page: `pages/<slug>.html`, a number added when
/// two screens fold to the same name.
#[must_use]
pub fn planned(screens: &[Screen]) -> Vec<Planned> {
    let mut out: Vec<Planned> = Vec::new();
    for screen in screens {
        let base = slug(&screen.name);
        let mut path = format!("pages/{base}.html");
        let mut n = 2;
        while out.iter().any(|p| p.path == path) {
            path = format!("pages/{base}-{n}.html");
            n += 1;
        }
        out.push(Planned {
            screen: screen.clone(),
            path,
        });
    }
    out
}

fn manifest(pages_json: Option<&str>) -> Result<Value, String> {
    let mut root: Value = match pages_json {
        Some(text) if !text.trim().is_empty() => {
            serde_json::from_str(text).map_err(|e| format!("pages.json: {e}"))?
        }
        _ => Value::Object(Map::new()),
    };
    let object = root.as_object_mut().ok_or("pages.json is not an object")?;
    object.entry("version").or_insert(Value::from(1));
    for list in ["pages", "components"] {
        let slot = object
            .entry(list)
            .or_insert_with(|| Value::Array(Vec::new()));
        if !slot.is_array() {
            return Err(format!("pages.json: {list} is not a list"));
        }
    }
    Ok(root)
}

/// `mock/pages.json` listing exactly these pages — made or brought up to
/// date by path, their other fields kept — any other page gone.
///
/// # Errors
///
/// The manifest is not JSON, or not shaped as contract N.
pub fn with_pages(pages_json: Option<&str>, pages: &[Planned]) -> Result<String, String> {
    let mut root = manifest(pages_json)?;
    let list = root
        .get_mut("pages")
        .and_then(Value::as_array_mut)
        .ok_or("pages.json: pages is not a list")?;
    list.retain(|p| {
        let path = p.get("path").and_then(Value::as_str);
        path.is_some_and(|path| pages.iter().any(|q| q.path == path))
    });
    for page in pages {
        let at = list
            .iter()
            .position(|p| p.get("path").and_then(Value::as_str) == Some(page.path.as_str()));
        let mut entry = at
            .and_then(|i| list.get(i))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        entry.insert("path".into(), Value::String(page.path.clone()));
        entry.insert("title".into(), Value::String(page.screen.name.clone()));
        entry.insert(
            "objective".into(),
            Value::String(page.screen.description.clone()),
        );
        entry
            .entry("components")
            .or_insert_with(|| Value::Array(Vec::new()));
        match at.and_then(|i| list.get_mut(i)) {
            Some(slot) => *slot = Value::Object(entry),
            None => list.push(Value::Object(entry)),
        }
    }
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    Ok(format!("{text}\n"))
}

/// `mock/index.html`: the proof of concept's pages, each a link. Plain
/// style: it is the entrance, not a page of the design.
#[must_use]
pub fn index_html(name: &str, pages: &[Planned]) -> String {
    let mut items = String::new();
    for page in pages {
        let _ = writeln!(
            items,
            "      <li><a href=\"{path}\">{title}</a><p>{objective}</p></li>",
            path = escape(&page.path),
            title = escape(&page.screen.name),
            objective = escape(&page.screen.description),
        );
    }
    format!(
        "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\">\n  \
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  \
         <title>{name}</title>\n  \
         <style>\n    body {{ font-family: system-ui, sans-serif; max-width: 48rem; \
         margin: 0 auto; padding: 2rem 1rem; }}\n    \
         ul {{ list-style: none; padding: 0; display: grid; gap: 1rem; }}\n    \
         li p {{ margin: 0.25rem 0 0; opacity: 0.7; }}\n  \
         </style>\n</head>\n<body>\n  <h1>{name}</h1>\n  <ul>\n{items}  </ul>\n</body>\n</html>\n",
        name = escape(name),
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The screens and their pages, as a session reads them: where each one
/// links to.
fn pages_block(all: &[Planned]) -> String {
    let mut text = String::from(
        "The proof of concept's screens and their pages — link the journeys to them by these \
         paths, relative to `mock/` (from a page, `other.html`):\n\n",
    );
    for p in all {
        let _ = writeln!(
            text,
            "- `{}` — **{}**: {}",
            p.path, p.screen.name, p.screen.description
        );
    }
    text
}

fn kit_block(kit: &str) -> String {
    if kit.trim().is_empty() {
        "The kit has no generated component yet: style the page with the tokens alone.\n\n"
            .to_string()
    } else {
        format!("The kit's components, one line each:\n\n{}\n\n", kit.trim())
    }
}

/// What every drawing session writes, and only that, and how a page is
/// built.
fn own_files(path: &str) -> String {
    let stem = slug(path.trim_start_matches("pages/").trim_end_matches(".html"));
    format!(
        "Write `mock/{path}` and the fixtures it reads, named `mock/fixtures/{stem}-<what>.json`, \
         with believable example data in the language of the brief — never a lorem ipsum. Write \
         nothing else: other sessions draw the other pages at the same time, and the factory \
         view lists the pages in `mock/pages.json` and writes `mock/index.html` itself — do not \
         edit them. Add the links between the pages that carry the journeys, so a human can \
         click through them. A page is a kind of screen: title it by the kind of screen it is, \
         never by the name or the number of one record of its example data, and give it the \
         record it shows through its fixture. The menu every page shares lists the product's \
         sections, the screens reachable from anywhere; a screen about one record is reached \
         from the list that holds it, never named in that menu. Give every region of the page — \
         its header, its navigation, its main area, each section, aside, list and form, its \
         footer — an `id` in kebab-case, unique on the page: the human pins requests on them, \
         and they are what an edit names. Build the page from the design system: its kit \
         components are dropped as `<div data-kit=\"<Name>\" data-props='{{…}}'></div>`, which \
         `../mock.js` fills — use one wherever it fits rather than writing that markup again. \
         Its markup may stay inline: it is cut into micro-frontends later. Say in one line what \
         the page shows.\n\n"
    )
}

/// The common tail of both prompts: the screens, the kit, the design
/// system, the product, the scope.
fn context(c: &Creation, kit: &str, tokens_css: &str, all: &[Planned]) -> String {
    let mut text = pages_block(all);
    text.push('\n');
    text.push_str(&kit_block(kit));
    text.push_str(&design_brief(tokens_css));
    text.push_str(&product(c));
    let _ = write!(
        text,
        "\nThe proof of concept's scope:\n\n{}\n",
        c.scope.trim()
    );
    text
}

/// The session that draws the first page, the one whose look every other
/// page follows. `rules` are the mock-up's own (`note_session::mockup_rules`),
/// `kit` the design system's components, one line each.
#[must_use]
pub fn reference_prompt(
    c: &Creation,
    rules: &str,
    kit: &str,
    tokens_css: &str,
    page: &Planned,
    all: &[Planned],
) -> String {
    let mut text = rules.to_string();
    let _ = write!(
        text,
        "\nDraw the first page of the proof of concept's mock-up: the screen **{name}** — \
         {description}. It is the **reference**: every other page is drawn after it, by \
         other sessions that copy its look — the layout, the navigation, the header, the \
         spacing, the way data is shown. Give it the care of a finished product.\n\n",
        name = page.screen.name,
        description = page.screen.description,
    );
    text.push_str(&own_files(&page.path));
    text.push_str(&context(c, kit, tokens_css, all));
    text
}

/// The session that draws one more page, on the reference's look.
#[must_use]
pub fn screen_prompt(
    c: &Creation,
    rules: &str,
    kit: &str,
    tokens_css: &str,
    page: &Planned,
    reference: &str,
    all: &[Planned],
) -> String {
    let mut text = rules.to_string();
    let _ = write!(
        text,
        "\nDraw one page of the proof of concept's mock-up: the screen **{name}** — \
         {description}. Read `mock/{reference}` first: it is the **reference** page, and this \
         one must look like it belongs beside it — the same layout, navigation, header, \
         spacing, components and way of showing data. Copy its shared parts as they are; \
         change what this screen needs. Do not edit the reference.\n\n",
        name = page.screen.name,
        description = page.screen.description,
    );
    text.push_str(&own_files(&page.path));
    text.push_str(&context(c, kit, tokens_css, all));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str = "## Goal\n\nProve it.\n\n## Screens\n\n\
        - **Invoices** — the list of invoices, filtered by status\n  \
          with a total at the bottom\n\
        - **Invoice detail**: one invoice, its lines, the pay button\n\
        * Settings - the account's preferences\n\
        1. **Écran d'accueil** — le tableau de bord\n\
        - **invoices** — a duplicate, ignored\n\n\
        ## Journeys\n\n- **Pay**: open, pay\n";

    #[test]
    fn the_screens_are_read_off_the_scope_s_section_only() {
        let found = screens(SCOPE);
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["Invoices", "Invoice detail", "Settings", "Écran d'accueil"]
        );
        assert_eq!(
            found[0].description,
            "the list of invoices, filtered by status with a total at the bottom"
        );
        assert_eq!(
            found[1].description,
            "one invoice, its lines, the pay button"
        );
        assert_eq!(found[2].description, "the account's preferences");
        assert_eq!(
            screens("## Goal\n\n- **Not** a screen\n"),
            Vec::<Screen>::new()
        );
        let french = screens("## Écrans\n\n- **Accueil** : le tableau\n");
        assert_eq!(french[0].name, "Accueil");
        assert_eq!(french[0].description, "le tableau");
        let plain = screens("## Screens\n- Home\nA paragraph about home.\n2) Help\n");
        assert_eq!(plain[0].name, "Home");
        assert_eq!(plain[0].description, "A paragraph about home.");
        assert_eq!(plain[1].name, "Help");
        assert_eq!(
            screens("## Screens\n\nfree text first\n"),
            Vec::<Screen>::new()
        );
        assert_eq!(screens("## Screens\n- **Open** bold\n")[0].name, "Open");
    }

    #[test]
    fn a_screen_gets_a_page_named_after_it_once() {
        assert_eq!(slug("Écran d'accueil"), "ecran-d-accueil");
        assert_eq!(slug("  Invoice / Détail  "), "invoice-detail");
        assert_eq!(slug("!!!"), "screen");
        assert_eq!(slug("Façade ñ Ü"), "facade-n-u");
        let twins = ["Home", "home!", "HOME"].map(|name| Screen {
            name: name.into(),
            description: String::new(),
        });
        let paths: Vec<String> = planned(&twins).into_iter().map(|p| p.path).collect();
        assert_eq!(
            paths,
            ["pages/home.html", "pages/home-2.html", "pages/home-3.html"]
        );
    }

    #[test]
    fn the_desk_lists_the_pages_itself_keeping_what_was_there() {
        let all = planned(&screens(SCOPE));
        let listed = with_pages(None, &all[..2]).expect("listed");
        let root: Value = serde_json::from_str(&listed).expect("json");
        assert_eq!(root["version"], 1);
        assert_eq!(root["pages"][0]["path"], "pages/invoices.html");
        assert_eq!(root["pages"][1]["title"], "Invoice detail");
        assert_eq!(root["components"], serde_json::json!([]));
        let kept = r#"{"version":1,"extra":true,"pages":[{"path":"pages/invoices.html","title":"Old","components":["Kept"]},{"path":"pages/gone.html","title":"Gone"}],"components":[]}"#;
        let again = with_pages(Some(kept), &all[..1]).expect("listed");
        let root: Value = serde_json::from_str(&again).expect("json");
        assert_eq!(root["extra"], true);
        assert_eq!(root["pages"].as_array().map(Vec::len), Some(1));
        assert_eq!(root["pages"][0]["title"], "Invoices");
        assert_eq!(root["pages"][0]["components"], serde_json::json!(["Kept"]));
        assert!(with_pages(Some("[]"), &all).is_err());
        assert!(with_pages(Some("nope"), &all).is_err());
        assert!(with_pages(Some(r#"{"pages":{}}"#), &all).is_err());
        assert!(listed.ends_with("}\n"));
    }

    #[test]
    fn the_index_lists_the_pages() {
        let all = planned(&screens(SCOPE));
        let html = index_html("Bills & <co>", &all);
        assert!(html.contains("<title>Bills &amp; &lt;co&gt;</title>"));
        assert!(html.contains("href=\"pages/invoices.html\">Invoices</a>"));
        assert!(
            !html.contains("var(--"),
            "no variable the design system may lack"
        );
        assert_eq!(escape("\"x\""), "&quot;x&quot;");
    }

    #[test]
    fn each_drawing_session_writes_its_own_page_and_follows_the_reference() {
        let c = Creation {
            name: "bills".into(),
            brief: "Pay invoices".into(),
            scope: SCOPE.into(),
            ..Creation::default()
        };
        let all = planned(&screens(SCOPE));
        let css = ":root { --space-1: 4px; }";
        let first = reference_prompt(&c, "RULES", "", css, &all[0], &all);
        assert!(first.starts_with("RULES"));
        assert!(first.contains("**reference**"));
        assert!(first.contains("`mock/pages/invoices.html`"));
        assert!(first.contains("mock/fixtures/invoices-<what>.json"));
        assert!(first.contains("`pages/invoice-detail.html` — **Invoice detail**"));
        assert!(first.contains("tokens alone"));
        assert!(first.contains("--space-1: 4px;"));
        assert!(first.contains("an `id` in kebab-case, unique on the page"));
        assert!(first.contains("never by the name or the number of one record"));
        assert!(first.contains("never named in that menu"));
        assert!(first.contains("Pay invoices"));
        assert!(first.ends_with("- **Pay**: open, pay\n"));
        let kit = reference_prompt(&c, "RULES", "Button — a button", "", &all[0], &all);
        assert!(kit.contains("Button — a button"));
        let other = screen_prompt(&c, "RULES", "", css, &all[1], &all[0].path, &all);
        assert!(other.contains("Read `mock/pages/invoices.html` first"));
        assert!(other.contains("`mock/pages/invoice-detail.html`"));
        assert!(other.contains("Do not edit the reference"));
        assert!(other.contains("--space-1: 4px;"));
    }
}
