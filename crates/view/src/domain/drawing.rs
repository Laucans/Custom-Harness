//! The proof of concept drawn screen by screen: the screens read off the
//! scope, the page each one gets, what moved between two scopes, the
//! manifest the desk keeps itself — sessions drawing in parallel never
//! write it — and the questions put to the sessions that draw and cut.
//!
//! Pure: texts in, texts out.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::domain::creation::{Creation, product};
use crate::domain::mf_gen::{fixture_path, fragment_path};

/// The headings the scope's screens may sit under: the one the scope
/// prompt asks for, and the French a brief written in French may bring.
const SCREENS_HEADINGS: [&str; 3] = ["screens", "écrans", "ecrans"];

/// One screen of the proof of concept, as the scope lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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

/// What moved between two lists of screens, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Screens the second list has and the first has not.
    pub added: Vec<String>,
    /// Screens the first list has and the second has not.
    pub removed: Vec<String>,
    /// Screens both have, described differently.
    pub changed: Vec<String>,
}

fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What moved from `before` to `after`: names compared without case, the
/// descriptions without their spacing.
#[must_use]
pub fn screens_changed(before: &[Screen], after: &[Screen]) -> Changes {
    let find = |list: &[Screen], name: &str| {
        list.iter()
            .find(|s| s.name.to_lowercase() == name.to_lowercase())
            .cloned()
    };
    let mut changes = Changes::default();
    for screen in after {
        match find(before, &screen.name) {
            None => changes.added.push(screen.name.clone()),
            Some(old) if words(&old.description) != words(&screen.description) => {
                changes.changed.push(screen.name.clone());
            }
            Some(_) => {}
        }
    }
    for screen in before {
        if find(after, &screen.name).is_none() {
            changes.removed.push(screen.name.clone());
        }
    }
    changes
}

/// The first page drawn, and the scope's screens it was drawn from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reference {
    /// The screen it shows.
    pub screen: String,
    /// Its page, relative to `mock/`.
    pub path: String,
    /// The scope's screens when it was drawn.
    pub screens: Vec<Screen>,
}

/// A page made of micro-frontends, and the fingerprint its file had right
/// after: a page whose file moved since is cut again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cut {
    /// The page, relative to `mock/`.
    pub path: String,
    /// Its fingerprint once cut.
    pub hash: String,
}

/// The pages to cut: those not cut yet, or whose file moved since.
#[must_use]
pub fn to_cut(pages: &[(String, String)], cut: &[Cut]) -> Vec<String> {
    pages
        .iter()
        .filter(|(path, hash)| !cut.iter().any(|c| &c.path == path && &c.hash == hash))
        .map(|(path, _)| path.clone())
        .collect()
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

fn written(root: &Value) -> Result<String, String> {
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(root).map_err(|e| e.to_string())?
    ))
}

fn list_mut<'a>(root: &'a mut Value, key: &str) -> Result<&'a mut Vec<Value>, String> {
    root.get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("pages.json: {key} is not a list"))
}

/// `mock/pages.json` with these pages listed — made or brought up to date
/// by path, their other fields kept — and the pages of `drop` gone.
///
/// # Errors
///
/// The manifest is not JSON, or not shaped as contract N.
pub fn with_pages(
    pages_json: Option<&str>,
    pages: &[Planned],
    drop: &[String],
) -> Result<String, String> {
    let mut root = manifest(pages_json)?;
    let list = list_mut(&mut root, "pages")?;
    list.retain(|p| {
        p.get("path")
            .and_then(Value::as_str)
            .is_none_or(|path| !drop.iter().any(|d| d == path))
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
    written(&root)
}

/// A micro-frontend as a cutting session reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CutComponent {
    /// Its name, `PascalCase`.
    pub name: String,
    /// Its bounded context.
    pub system: String,
    /// What it shows and lets the user do.
    pub description: String,
    /// The Capabilities it would read.
    pub needs: Vec<String>,
    /// The `DataCapabilities` it would trigger.
    pub actions: Vec<String>,
    /// Its props, `name: type`.
    pub props: BTreeMap<String, String>,
    /// It reads a fixture of its own, `fixtures/<Name>.json`.
    pub fixture: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CutAnswer {
    components: Vec<CutComponent>,
}

/// The micro-frontends a cutting session reported, those with a name; an
/// answer that carries none reads as none — the page itself says which it
/// injects.
#[must_use]
pub fn parse_cut(answer: &str) -> Vec<CutComponent> {
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Vec::new();
    };
    answer
        .get(start..=end)
        .and_then(|object| serde_json::from_str::<CutAnswer>(object).ok())
        .map(|a| {
            a.components
                .into_iter()
                .filter(|c| !c.name.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `mock/pages.json` once a page is cut: the page lists the
/// micro-frontends it injects, and each of them is listed — a reported one
/// made or brought up to date, one only injected listed by its fragment if
/// nothing lists it yet.
///
/// # Errors
///
/// The manifest is not JSON, or not shaped as contract N.
pub fn with_cut(
    pages_json: Option<&str>,
    page: &str,
    injected: &[String],
    reported: &[CutComponent],
) -> Result<String, String> {
    let mut root = manifest(pages_json)?;
    let pages = list_mut(&mut root, "pages")?;
    if let Some(entry) = pages
        .iter_mut()
        .find(|p| p.get("path").and_then(Value::as_str) == Some(page))
        .and_then(Value::as_object_mut)
    {
        entry.insert("components".into(), serde_json::json!(injected));
    }
    let components = list_mut(&mut root, "components")?;
    for name in injected {
        let at = components
            .iter()
            .position(|c| c.get("name").and_then(Value::as_str) == Some(name.as_str()));
        let report = reported.iter().find(|c| &c.name == name);
        let mut entry = match (at.and_then(|i| components.get(i)), report) {
            (Some(existing), None) => {
                if existing.is_object() {
                    continue;
                }
                Map::new()
            }
            (existing, _) => existing
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        };
        entry.insert("name".into(), Value::String(name.clone()));
        entry.insert("fragment".into(), Value::String(fragment_path(name)));
        if let Some(c) = report {
            entry.insert("system".into(), Value::String(c.system.clone()));
            entry.insert("description".into(), Value::String(c.description.clone()));
            entry.insert("needs".into(), serde_json::json!(c.needs));
            entry.insert("actions".into(), serde_json::json!(c.actions));
            entry.insert("props".into(), serde_json::json!(c.props));
            if c.fixture {
                entry.insert("fixture".into(), Value::String(fixture_path(name)));
            }
        }
        match at.and_then(|i| components.get_mut(i)) {
            Some(slot) => *slot = Value::Object(entry),
            None => components.push(Value::Object(entry)),
        }
    }
    written(&root)
}

/// `mock/index.html`: the proof of concept's pages, each a link, on the
/// design system's tokens.
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
         <title>{name}</title>\n  <link rel=\"stylesheet\" href=\"design/tokens.css\">\n  \
         <style>\n    body {{ font-family: var(--font-body, system-ui, sans-serif); \
         background: var(--color-background, #fff); color: var(--color-text, #111); \
         max-width: 48rem; margin: 0 auto; padding: 2rem 1rem; }}\n    \
         ul {{ list-style: none; padding: 0; display: grid; gap: 1rem; }}\n    \
         li p {{ margin: 0.25rem 0 0; color: var(--color-text-muted, #555); }}\n  \
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

/// What every drawing session writes, and only that.
fn own_files(path: &str) -> String {
    let stem = slug(path.trim_start_matches("pages/").trim_end_matches(".html"));
    format!(
        "Write `mock/{path}` and the fixtures it reads, named `mock/fixtures/{stem}-<what>.json`, \
         with believable example data in the language of the brief — never a lorem ipsum. Write \
         nothing else: other sessions draw the other pages at the same time, and the factory \
         view lists the pages in `mock/pages.json` and writes `mock/index.html` itself — do not \
         edit them. Build the page from the design system: `mock/design/tokens.css` holds its \
         variables (`var(--…)`, never a colour or a size written by hand), and its kit components \
         are dropped as `<div data-kit=\"<Name>\" data-props='{{…}}'></div>`, which `../mock.js` \
         fills — use one wherever it fits rather than writing that markup again. Its markup may \
         stay inline: it is cut into micro-frontends later. Say in one line what the page shows.\n\n"
    )
}

/// The session that draws the first page, the one whose look every other
/// page follows. `existing`: the page is there from an earlier scope, and
/// is rewritten for the screen as it is now, keeping its look.
#[must_use]
pub fn reference_prompt(
    c: &Creation,
    rules: &str,
    kit: &str,
    page: &Planned,
    all: &[Planned],
    existing: bool,
) -> String {
    let mut text = rules.to_string();
    if existing {
        let _ = write!(
            text,
            "\n`mock/{path}` was drawn for an earlier version of the scope. Rewrite it for the \
             screen **{name}** as the scope describes it now — {description} — keeping its look \
             exactly: the other pages are drawn after it, on its model.\n\n",
            path = page.path,
            name = page.screen.name,
            description = page.screen.description,
        );
    } else {
        let _ = write!(
            text,
            "\nDraw the first page of the proof of concept's mock-up: the screen **{name}** — \
             {description}. It is the **reference**: every other page is drawn after it, by \
             other sessions that copy its look — the layout, the navigation, the header, the \
             spacing, the way data is shown. Give it the care of a finished product.\n\n",
            name = page.screen.name,
            description = page.screen.description,
        );
    }
    text.push_str(&own_files(&page.path));
    text.push_str(&pages_block(all));
    text.push('\n');
    text.push_str(&kit_block(kit));
    text.push_str(&product(c));
    let _ = write!(
        text,
        "\nThe proof of concept's scope:\n\n{}\n",
        c.scope.trim()
    );
    text
}

/// The session that draws one more page, on the reference's model.
#[must_use]
pub fn screen_prompt(
    c: &Creation,
    rules: &str,
    kit: &str,
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
    text.push_str(&pages_block(all));
    text.push('\n');
    text.push_str(&kit_block(kit));
    text.push_str(&product(c));
    let _ = write!(
        text,
        "\nThe proof of concept's scope:\n\n{}\n",
        c.scope.trim()
    );
    text
}

/// The session that cuts one page into micro-frontends, keeping what the
/// human sees. `existing` lists the micro-frontends already made, one line
/// each: reused, never written again.
#[must_use]
pub fn cut_prompt(c: &Creation, rules: &str, kit: &str, page: &str, existing: &str) -> String {
    let mut text = rules.to_string();
    let _ = write!(
        text,
        "\nMake the page `mock/{page}` again of **micro-frontends**, as the product's \
         architecture wants it, without changing what the human sees. Cut it into the Micro-UIs \
         it is made of — a unit a user recognises (an invoice list, an upload form, a balance), \
         named in PascalCase, its system the bounded context it belongs to — and for each new \
         one write its fragment `mock/components/<Name>.html` (its markup, its style on the design \
         system's variables, its script finding its root as \
         `document.currentScript.closest('[data-component]')`, its data read from \
         `mock/fixtures/<name-in-kebab-case>.json` — `InvoiceList` reads `fixtures/invoice-list.json` —, the kit's components kept as `data-kit` elements). Then \
         rewrite the page as layout only: its regions, and in them \
         `<div data-component=\"<Name>\" data-props='{{…}}'></div>` for each micro-frontend, with \
         `<script src=\"../mock.js\"></script>` — no business markup left inline. Keep its look, \
         texts, links and example data as they are. Other sessions cut the other pages at the \
         same time: write only this page, its new micro-frontends and their fixtures, and do \
         **not** edit `mock/pages.json` — the factory view lists them from your answer.\n\n",
    );
    if existing.trim().is_empty() {
        text.push_str("No micro-frontend exists yet.\n\n");
    } else {
        let _ = write!(
            text,
            "These micro-frontends exist already: inject one wherever it fits, as it is — never \
             write its fragment again:\n\n{}\n\n",
            existing.trim()
        );
    }
    text.push_str(&kit_block(kit));
    text.push_str(
        "End your answer with one JSON object listing the micro-frontends you **made** for this \
         page: {\"components\": [{\"name\": \"…\", \"system\": \"…\", \"description\": \"…\", \
         \"needs\": [\"<system>.<verbThing>\"], \"actions\": [\"<system>.<verbThing>\"], \
         \"props\": {\"name\": \"type\"}, \"fixture\": true}]} — `needs` the Capabilities it would \
         read, `actions` the DataCapabilities a user action would trigger.\n\n",
    );
    text.push_str(&product(c));
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
        let twins = [
            Screen {
                name: "Home".into(),
                description: String::new(),
            },
            Screen {
                name: "home!".into(),
                description: String::new(),
            },
            Screen {
                name: "HOME".into(),
                description: String::new(),
            },
        ];
        let paths: Vec<String> = planned(&twins).into_iter().map(|p| p.path).collect();
        assert_eq!(
            paths,
            ["pages/home.html", "pages/home-2.html", "pages/home-3.html"]
        );
    }

    #[test]
    fn what_moved_between_two_scopes_is_named() {
        let before = screens(SCOPE);
        let after = screens(
            "## Screens\n- **invoices** — the list of invoices,   filtered by status with a total at the bottom\n\
             - **Invoice detail** — one invoice and a refund button\n- **Reports** — totals\n",
        );
        let changes = screens_changed(&before, &after);
        assert_eq!(changes.added, ["Reports"]);
        assert_eq!(changes.changed, ["Invoice detail"]);
        assert_eq!(changes.removed, ["Settings", "Écran d'accueil"]);
        assert_eq!(screens_changed(&before, &before), Changes::default());
    }

    #[test]
    fn a_page_is_cut_again_when_its_file_moved() {
        let cut = [
            Cut {
                path: "pages/a.html".into(),
                hash: "1".into(),
            },
            Cut {
                path: "pages/b.html".into(),
                hash: "2".into(),
            },
        ];
        let pages = [
            ("pages/a.html".to_string(), "1".to_string()),
            ("pages/b.html".to_string(), "9".to_string()),
            ("pages/c.html".to_string(), "3".to_string()),
        ];
        assert_eq!(to_cut(&pages, &cut), ["pages/b.html", "pages/c.html"]);
        assert_eq!(to_cut(&pages[..1], &cut), Vec::<String>::new());
    }

    #[test]
    fn the_desk_lists_the_pages_itself_keeping_what_was_there() {
        let all = planned(&screens(SCOPE));
        let listed = with_pages(None, &all[..2], &[]).expect("listed");
        let root: Value = serde_json::from_str(&listed).expect("json");
        assert_eq!(root["version"], 1);
        assert_eq!(root["pages"][0]["path"], "pages/invoices.html");
        assert_eq!(root["pages"][1]["title"], "Invoice detail");
        assert_eq!(root["components"], serde_json::json!([]));
        let kept = r#"{"version":1,"extra":true,"pages":[{"path":"pages/invoices.html","title":"Old","components":["Kept"]},{"path":"pages/gone.html","title":"Gone"}],"components":[]}"#;
        let again =
            with_pages(Some(kept), &all[..1], &["pages/gone.html".to_string()]).expect("listed");
        let root: Value = serde_json::from_str(&again).expect("json");
        assert_eq!(root["extra"], true);
        assert_eq!(root["pages"].as_array().map(Vec::len), Some(1));
        assert_eq!(root["pages"][0]["title"], "Invoices");
        assert_eq!(root["pages"][0]["components"], serde_json::json!(["Kept"]));
        assert!(with_pages(Some("[]"), &all, &[]).is_err());
        assert!(with_pages(Some("nope"), &all, &[]).is_err());
        assert!(with_pages(Some(r#"{"pages":{}}"#), &all, &[]).is_err());
        assert!(listed.ends_with("}\n"));
    }

    #[test]
    fn a_cut_page_lists_what_it_injects_and_each_one_is_listed_once() {
        let base = with_pages(None, &planned(&screens(SCOPE)), &[]).expect("listed");
        let reported = parse_cut(
            "Made two.\n```json\n{\"components\": [{\"name\": \"InvoiceList\", \"system\": \"billing\", \
             \"description\": \"the invoices\", \"needs\": [\"billing.listInvoices\"], \"actions\": [], \
             \"props\": {\"status\": \"string\"}, \"fixture\": true}, {\"name\": \" \"}]}\n```",
        );
        assert_eq!(reported.len(), 1);
        let names = vec!["InvoiceList".to_string(), "Header".to_string()];
        let cut = with_cut(Some(&base), "pages/invoices.html", &names, &reported).expect("cut");
        let root: Value = serde_json::from_str(&cut).expect("json");
        assert_eq!(root["pages"][0]["components"], serde_json::json!(names));
        assert_eq!(
            root["components"][0]["fragment"],
            "components/InvoiceList.html"
        );
        assert_eq!(
            root["components"][0]["fixture"],
            "fixtures/invoice-list.json"
        );
        assert_eq!(
            root["components"][0]["needs"],
            serde_json::json!(["billing.listInvoices"])
        );
        assert_eq!(root["components"][0]["props"]["status"], "string");
        assert_eq!(root["components"][1]["name"], "Header");
        assert!(root["components"][1].get("fixture").is_none());
        // Cut again from another page: a listed one only injected stays as it is.
        let again =
            with_cut(Some(&cut), "pages/invoice-detail.html", &names[..1], &[]).expect("cut");
        let root: Value = serde_json::from_str(&again).expect("json");
        assert_eq!(root["components"].as_array().map(Vec::len), Some(2));
        assert_eq!(root["components"][0]["system"], "billing");
        assert_eq!(parse_cut("no json"), Vec::<CutComponent>::new());
        assert_eq!(parse_cut("{not json}"), Vec::<CutComponent>::new());
    }

    #[test]
    fn the_index_lists_the_pages_on_the_tokens() {
        let all = planned(&screens(SCOPE));
        let html = index_html("Bills & <co>", &all);
        assert!(html.contains("<title>Bills &amp; &lt;co&gt;</title>"));
        assert!(html.contains("href=\"pages/invoices.html\">Invoices</a>"));
        assert!(html.contains("design/tokens.css"));
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
        let first = reference_prompt(&c, "RULES", "", &all[0], &all, false);
        assert!(first.starts_with("RULES"));
        assert!(first.contains("**reference**"));
        assert!(first.contains("`mock/pages/invoices.html`"));
        assert!(first.contains("mock/fixtures/invoices-<what>.json"));
        assert!(first.contains("do not \n") || first.contains("do not"));
        assert!(first.contains("`pages/invoice-detail.html` — **Invoice detail**"));
        assert!(first.contains("tokens alone"));
        assert!(first.contains("Pay invoices"));
        let again = reference_prompt(&c, "RULES", "Button — a button", &all[0], &all, true);
        assert!(again.contains("keeping its look"));
        assert!(again.contains("Button — a button"));
        let other = screen_prompt(&c, "RULES", "", &all[1], &all[0].path, &all);
        assert!(other.contains("Read `mock/pages/invoices.html` first"));
        assert!(other.contains("`mock/pages/invoice-detail.html`"));
        assert!(other.contains("Do not edit the reference"));
        let cut = cut_prompt(&c, "RULES", "", "pages/invoices.html", "");
        assert!(cut.contains("`mock/pages/invoices.html`"));
        assert!(cut.contains("No micro-frontend exists yet"));
        assert!(cut.contains("do **not** edit `mock/pages.json`"));
        let reuse = cut_prompt(&c, "RULES", "", "pages/x.html", "- Header (shell)");
        assert!(reuse.contains("- Header (shell)"));
        assert!(reuse.contains("never write its fragment again"));
    }
}
