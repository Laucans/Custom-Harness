//! The proof of concept's mock-up, drawn in two hands: a planner (a large
//! model, once) decides every page and every micro-frontend — what each
//! shows, what it reads and triggers, the kit's components it is built of,
//! the data its fixture holds — and writers (a faster model, many at once)
//! write each micro-frontend and each page from that plan alone. The plan
//! is the manifest (`mock/pages.json`, contract N) before any file exists,
//! so every writer works in parallel and the page counts what has landed.
//!
//! Pure: the questions are asked, and the files written, by the desk.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::Deserialize;

use super::creation::Creation;
use super::mf_gen::{self, Spec};
use super::mockup::Manifest;
use crate::ports::{MockComponent, MockPage};

/// The mock-up as the planner decides it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MockPlan {
    /// One page per screen of the scope.
    pub pages: Vec<PlannedPage>,
    /// Every micro-frontend the pages inject, each once.
    pub components: Vec<PlannedComponent>,
    /// The example world every writer shares: each entity the fixtures and
    /// the pages' props name, with its fixed id — so a page hands a
    /// micro-frontend an id its fixture has.
    pub world: String,
}

/// One page of the plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PlannedPage {
    /// `pages/<name>.html`.
    pub path: String,
    /// Its title.
    pub title: String,
    /// What the user does there.
    pub objective: String,
    /// The micro-frontends it injects, by name, in the order they show.
    pub components: Vec<String>,
    /// The pages it links to, by path: the journeys.
    pub links: Vec<String>,
    /// Its layout, in words: its regions and what sits in each.
    pub layout: String,
}

/// One micro-frontend of the plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PlannedComponent {
    /// `PascalCase`.
    pub name: String,
    /// Its bounded context.
    pub system: String,
    /// What it shows and lets the user do.
    pub description: String,
    /// The Capabilities it reads, `<system>.<verbThing>`.
    pub needs: Vec<String>,
    /// The `DataCapabilities` it triggers.
    pub actions: Vec<String>,
    /// What a page hands it.
    pub props: BTreeMap<String, String>,
    /// The kit's components it is built of, by name.
    pub uses: Vec<String>,
    /// What its fixture holds: the example data, in a sentence or two.
    pub data: String,
}

impl PlannedComponent {
    /// The spec its writer is handed.
    #[must_use]
    pub fn spec(&self) -> Spec {
        Spec {
            name: self.name.clone(),
            system: self.system.clone(),
            description: self.description.clone(),
            needs: self.needs.clone(),
            actions: self.actions.clone(),
            props: self.props.clone(),
        }
    }
}

/// The planner's question: the product, its scope, the kit — answered with
/// the whole plan as one JSON object.
#[must_use]
pub fn plan_prompt(c: &Creation, kit_index: &str) -> String {
    let mut text = String::from(
        "You plan the mock-up of a proof of concept, for a vertical-slice architecture: every \
         screen is a static page made of micro-frontends, and other, faster hands will write each \
         page and each micro-frontend from your plan alone, all at once — so the plan must say \
         everything they need, and agree with itself.\n\n\
         Plan one page per screen of the scope (`pages/<kebab-name>.html`), each with its title, \
         its objective (what the user does there), the micro-frontends it injects in the order \
         they show, the pages it links to (the journeys a human clicks through) and its layout in \
         words (its regions, and which micro-frontend sits in each). Plan every micro-frontend \
         once, even when several pages inject it: a unit a user recognises (an invoice list, an \
         upload form, a balance), named in PascalCase, its system the bounded context it belongs \
         to, what it shows and lets the user do, the Capabilities it reads (`needs`) and the \
         DataCapabilities it triggers (`actions`), named `<system>.<verbThing>`, the props a page \
         hands it, the kit's components it is built of (`uses`, by their exact names below), and \
         the example data its fixture holds (`data`: believable, in the language of the brief — \
         never a lorem ipsum). Then fix the example world they all share (`world`): every entity \
         a fixture holds or a page hands as a prop — the campaign, its sessions, its people, its \
         places, its media… — each with its fixed id, one line each (`cmp-1 — the campaign …`): \
         the writers never see each other, and only these ids let a page's props find a \
         fixture's rows.\n\n\
         Answer with one JSON object and nothing else: {\"pages\": [{\"path\": \"pages/home.html\", \
         \"title\": \"…\", \"objective\": \"…\", \"components\": [\"Name\"], \"links\": \
         [\"pages/other.html\"], \"layout\": \"…\"}], \"components\": [{\"name\": \"Name\", \
         \"system\": \"…\", \"description\": \"…\", \"needs\": [\"system.readThing\"], \"actions\": \
         [\"system.doThing\"], \"props\": {\"id\": \"ID\"}, \"uses\": [\"Button\"], \"data\": \
         \"…\"}], \"world\": \"cmp-1 — …\"}.\n\n",
    );
    if kit_index.trim().is_empty() {
        text.push_str("The kit has no generated component yet: `uses` stays empty.\n\n");
    } else {
        let _ = write!(
            text,
            "The kit's components, one line each:\n\n{}\n\n",
            kit_index.trim()
        );
    }
    let _ = write!(text, "What the application does:\n\n{}\n", c.brief.trim());
    let answered: Vec<_> = c.questions.iter().filter(|q| q.answered()).collect();
    if !answered.is_empty() {
        text.push_str("\nThe decisions:\n\n");
        for q in answered {
            let _ = writeln!(
                text,
                "- {} — {}",
                q.title.trim(),
                q.answer.trim().replace('\n', " ")
            );
        }
    }
    let _ = write!(
        text,
        "\nThe proof of concept's scope:\n\n{}\n",
        c.scope.trim()
    );
    text
}

/// The plan out of the planner's answer, kept to what agrees with itself:
/// pages under `pages/`, micro-frontends named in `PascalCase`, each once;
/// a page's micro-frontends and links to what the plan has; a
/// micro-frontend's `uses` to the kit's names.
///
/// # Errors
///
/// No JSON object, one that does not read, no page or no micro-frontend.
pub fn parse_plan(answer: &str, kit: &[String]) -> Result<MockPlan, String> {
    let start = answer.find('{').ok_or("the planner answered no plan")?;
    let end = answer.rfind('}').ok_or("the planner answered no plan")?;
    let object = answer
        .get(start..=end)
        .ok_or("the planner answered no plan")?;
    let read: MockPlan =
        serde_json::from_str(object).map_err(|e| format!("the plan is not readable: {e}"))?;
    let mut seen = BTreeSet::new();
    let components: Vec<PlannedComponent> = read
        .components
        .into_iter()
        .filter(|c| pascal(&c.name) && seen.insert(c.name.clone()))
        .map(|mut c| {
            c.uses.retain(|u| kit.contains(u));
            c
        })
        .collect();
    let mut paths = BTreeSet::new();
    let pages: Vec<PlannedPage> = read
        .pages
        .into_iter()
        .filter(|p| page_path(&p.path) && paths.insert(p.path.clone()))
        .collect();
    let pages: Vec<PlannedPage> = pages
        .into_iter()
        .map(|mut p| {
            p.components.retain(|n| seen.contains(n));
            p.links.retain(|l| paths.contains(l) && *l != p.path);
            p
        })
        .collect();
    if pages.is_empty() {
        return Err("the plan has no page".to_string());
    }
    if components.is_empty() {
        return Err("the plan has no micro-frontend".to_string());
    }
    Ok(MockPlan {
        pages,
        components,
        world: read.world.trim().to_string(),
    })
}

/// `PascalCase`: a capital, then letters and digits.
fn pascal(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

/// `pages/<name>.html`, no folder below.
fn page_path(path: &str) -> bool {
    path.strip_prefix("pages/")
        .and_then(|rest| rest.strip_suffix(".html"))
        .is_some_and(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// The plan as `mock/pages.json` writes it (contract N), before any file
/// exists: each micro-frontend's fragment and fixture where its writer
/// puts them.
///
/// # Errors
///
/// Never in practice: plain data.
pub fn manifest(plan: &MockPlan) -> Result<String, String> {
    let manifest = Manifest {
        pages: plan
            .pages
            .iter()
            .map(|p| MockPage {
                path: p.path.clone(),
                title: p.title.clone(),
                objective: p.objective.clone(),
                components: p.components.clone(),
                ..MockPage::default()
            })
            .collect(),
        components: plan
            .components
            .iter()
            .map(|c| MockComponent {
                name: c.name.clone(),
                system: c.system.clone(),
                description: c.description.clone(),
                fragment: mf_gen::fragment_path(&c.name),
                fixture: vec![mf_gen::fixture_path(&c.name)],
                needs: c.needs.clone(),
                actions: c.actions.clone(),
                props: c.props.clone(),
                ..MockComponent::default()
            })
            .collect(),
    };
    let mut value = serde_json::to_value(&manifest).map_err(|e| e.to_string())?;
    if let Some(object) = value.as_object_mut() {
        object.insert("version".to_string(), serde_json::json!(1));
    }
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

/// What a micro-frontend's writer is told of its units and of its fixture:
/// the planner's words, standing in for Capabilities not built yet, and the
/// world every fixture shares.
#[must_use]
pub fn units_of(component: &PlannedComponent, world: &str) -> String {
    let mut text = String::new();
    if !component.data.trim().is_empty() {
        let _ = writeln!(text, "Its fixture holds: {}", component.data.trim());
    }
    if !world.trim().is_empty() {
        let _ = write!(
            text,
            "Its fixture's rows are of this example world, with these ids exactly — the pages hand them as props:\n{}\n",
            world.trim()
        );
    }
    text
}

/// The design system's variables a text uses and the design system has
/// not — those the text itself defines (`--cols: 3`) aside.
#[must_use]
pub fn unknown_tokens(text: &str, tokens: &[String]) -> Vec<String> {
    let defined: BTreeSet<&str> = text
        .split([';', '{', '\n', '"', '\''])
        .filter_map(|decl| decl.trim().split_once(':'))
        .map(|(name, _)| name.trim())
        .filter(|name| name.starts_with("--"))
        .collect();
    let mut unknown = BTreeSet::new();
    for (at, _) in text.match_indices("var(--") {
        let name: String = text
            .get(at + 4..)
            .unwrap_or_default()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !tokens.contains(&name) && !defined.contains(name.as_str()) {
            unknown.insert(name);
        }
    }
    unknown.into_iter().collect()
}

/// A writer's answer checked against the design system: refused, with the
/// variables it may use, when it uses one the design system has not. No
/// design system to check against: nothing is refused.
///
/// # Errors
///
/// The variables it uses the design system has not.
pub fn on_tokens(text: &str, tokens: &[String]) -> Result<(), String> {
    if tokens.is_empty() {
        return Ok(());
    }
    let unknown = unknown_tokens(text, tokens);
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "it uses variables the design system has not: {}; the design system's are: {}",
        unknown.join(", "),
        tokens.join(", ")
    ))
}

/// A writer asked again: its question, and why its last answer was refused.
#[must_use]
pub fn retry_prompt(prompt: &str, why: &str) -> String {
    format!(
        "{prompt}\n\nYour last answer was refused: {why}. Answer again, the whole answer, with that corrected.\n"
    )
}

/// A page's writer's question: the page's plan, its micro-frontends' specs,
/// the pages it links to — answered with the page alone.
#[must_use]
pub fn page_prompt(c: &Creation, plan: &MockPlan, page: &PlannedPage, tokens: &[String]) -> String {
    let mut text = String::from(
        "You write one page of a product's static mock-up: plain HTML, no build, no backend. The \
         page is layout only — its regions, headings and the links of the journeys — and its \
         content is micro-frontends written by others: inject each one as `<div \
         data-component=\"<Name>\" data-props='{…}'></div>` in its region, with the props it \
         expects, and write none of its markup yourself. In `<head>`: `<link rel=\"stylesheet\" \
         href=\"../design/tokens.css\">` and `<link rel=\"stylesheet\" href=\"../kit/kit.css\">`; \
         style the layout through the design system's variables (`var(--…)`), never a colour or \
         a size written by hand — the page's own background, text colour and font included, set \
         on `body` from those variables. At the end of `<body>`: `<script src=\"../mock.js\"></script>`. \
         Link the other pages by their relative path (`other.html`). Write its texts in the \
         language of the brief. Answer ONLY a ```html block with the whole page.\n\n",
    );
    let _ = write!(
        text,
        "The application: {}\n\nThe page `{}` — {}: {}\nIts layout: {}\n",
        c.brief.trim().lines().next().unwrap_or_default(),
        page.path,
        page.title,
        page.objective,
        page.layout.trim()
    );
    text.push_str("\nIts micro-frontends, in the order they show:\n\n");
    for name in &page.components {
        if let Some(m) = plan.components.iter().find(|m| &m.name == name) {
            let props: Vec<String> = m.props.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            let _ = writeln!(
                text,
                "- `{}` — {} — props: {}",
                m.name,
                m.description.trim(),
                if props.is_empty() {
                    "none".to_string()
                } else {
                    props.join(", ")
                }
            );
        }
    }
    if tokens.is_empty() {
        text.push_str("\nThe design system has no variable yet: use none.\n");
    } else {
        let _ = write!(
            text,
            "\nThe design system's variables — use these and no other:\n{}\n",
            tokens.join(", ")
        );
    }
    if !plan.world.trim().is_empty() {
        let _ = write!(
            text,
            "\nThe example world: every id a prop hands is one of these, exactly:\n{}\n",
            plan.world.trim()
        );
    }
    if !page.links.is_empty() {
        text.push_str("\nThe pages it links to:\n\n");
        for link in &page.links {
            if let Some(other) = plan.pages.iter().find(|p| &p.path == link) {
                let _ = writeln!(
                    text,
                    "- `{}` — {}",
                    link.trim_start_matches("pages/"),
                    other.title
                );
            }
        }
    }
    text
}

/// The page out of its writer's answer: the HTML, fenced or not, checked to
/// inject every micro-frontend the plan gives it.
///
/// # Errors
///
/// No page in the answer, or a planned micro-frontend not injected.
pub fn page_of(answer: &str, page: &PlannedPage) -> Result<String, String> {
    let html = answer
        .split("```html")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap_or(answer)
        .trim()
        .to_string();
    if !html.to_lowercase().contains("<body") {
        return Err(format!("{}: the writer answered no page", page.path));
    }
    let missing: Vec<&str> = page
        .components
        .iter()
        .filter(|n| !html.contains(&format!("data-component=\"{n}\"")))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "{}: {} not injected",
            page.path,
            missing.join(", ")
        ));
    }
    Ok(html)
}

/// `mock/index.html`: every page of the plan, linked — written without a
/// question.
#[must_use]
pub fn index_html(c: &Creation, plan: &MockPlan) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut items = String::new();
    for p in &plan.pages {
        let _ = writeln!(
            items,
            "      <li><a href=\"{}\">{}</a> — {}</li>",
            esc(&p.path),
            esc(&p.title),
            esc(&p.objective)
        );
    }
    format!(
        "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\">\n  <title>{name} — mock-up</title>\n  \
         <link rel=\"stylesheet\" href=\"design/tokens.css\">\n</head>\n<body>\n  <h1>{name}</h1>\n  \
         <ul>\n{items}  </ul>\n</body>\n</html>\n",
        name = esc(&c.name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"Here: {"pages": [
        {"path": "pages/home.html", "title": "Home", "objective": "see the campaigns", "components": ["CampaignList", "Ghost"], "links": ["pages/campaign.html", "pages/nowhere.html", "pages/home.html"], "layout": "a list"},
        {"path": "pages/campaign.html", "title": "Campaign", "components": ["CampaignList"]},
        {"path": "pages/home.html", "title": "again"},
        {"path": "home.html", "title": "astray"},
        {"path": "pages/a/b.html", "title": "deep"}
    ], "components": [
        {"name": "CampaignList", "system": "campagne", "description": "the campaigns", "needs": ["campagne.lister"], "actions": ["campagne.creer"], "props": {"id": "ID"}, "uses": ["Card", "Unknown"], "data": "three campaigns"},
        {"name": "CampaignList", "description": "twice"},
        {"name": "lower"}
    ], "world": " cmp-1 — the campaign "}"#;

    fn plan() -> MockPlan {
        parse_plan(ANSWER, &["Card".to_string()]).expect("read")
    }

    #[test]
    fn the_plan_is_kept_to_what_agrees_with_itself() {
        for (path, ok) in [
            ("pages/a-b.html", true),
            ("pages/a_b.html", true),
            ("pages/a b.html", false),
            ("pages/.html", false),
            ("pages/a.htm", false),
        ] {
            assert_eq!(page_path(path), ok, "{path}");
        }
        let p = plan();
        assert_eq!(
            p.pages.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(),
            vec!["pages/home.html", "pages/campaign.html"]
        );
        assert_eq!(
            p.pages[0].components,
            vec!["CampaignList".to_string()],
            "a micro-frontend the plan has not is dropped"
        );
        assert_eq!(
            p.pages[0].links,
            vec!["pages/campaign.html".to_string()],
            "no link to nowhere, nor to itself"
        );
        assert_eq!(p.components.len(), 1, "each once, in PascalCase");
        assert_eq!(
            p.components[0].uses,
            vec!["Card".to_string()],
            "the kit's names only"
        );
        assert_eq!(
            p.components[0].spec().needs,
            vec!["campagne.lister".to_string()]
        );
        assert_eq!(p.world, "cmp-1 — the campaign");
        assert!(parse_plan("no plan", &[]).is_err());
        assert!(parse_plan("{\"pages\": 3}", &[]).is_err());
        assert!(parse_plan(r#"{"pages": [], "components": [{"name": "A"}]}"#, &[]).is_err());
        assert!(
            parse_plan(
                r#"{"pages": [{"path": "pages/a.html"}], "components": []}"#,
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn the_manifest_says_where_each_writer_puts_its_files() {
        let manifest: serde_json::Value =
            serde_json::from_str(&manifest(&plan()).expect("json")).expect("json");
        assert_eq!(manifest["version"], 1);
        assert_eq!(manifest["pages"][0]["title"], "Home");
        assert_eq!(manifest["pages"][0]["objective"], "see the campaigns");
        assert_eq!(manifest["components"][0]["description"], "the campaigns");
        assert_eq!(
            manifest["components"][0]["needs"],
            serde_json::json!(["campagne.lister"])
        );
        assert_eq!(
            manifest["components"][0]["props"],
            serde_json::json!({"id": "ID"})
        );
        assert_eq!(manifest["pages"][0]["components"][0], "CampaignList");
        assert_eq!(
            manifest["components"][0]["fragment"],
            "components/CampaignList.html"
        );
        assert_eq!(
            manifest["components"][0]["fixture"],
            "fixtures/campaign-list.json"
        );
        let read =
            super::super::mockup::manifest_of(&manifest.to_string()).expect("contract N reads it");
        assert_eq!(
            read.components[0].actions,
            vec!["campagne.creer".to_string()]
        );
        assert_eq!(
            units_of(&plan().components[0], " "),
            "Its fixture holds: three campaigns\n"
        );
        assert_eq!(units_of(&PlannedComponent::default(), ""), "");
        let with = units_of(&PlannedComponent::default(), " cmp-1 — the campaign ");
        assert!(
            with.ends_with("the pages hand them as props:\ncmp-1 — the campaign\n"),
            "{with}"
        );
    }

    #[test]
    fn a_page_is_read_off_its_answer_with_every_micro_frontend_injected() {
        let p = plan();
        let page = &p.pages[0];
        let good = "Sure:\n```html\n<html><body><div data-component=\"CampaignList\"></div></body></html>\n```\nDone.";
        assert_eq!(
            page_of(good, page).expect("read"),
            "<html><body><div data-component=\"CampaignList\"></div></body></html>"
        );
        assert!(
            page_of(
                "<html><BODY><div data-component=\"CampaignList\"></div></BODY></html>",
                page
            )
            .is_ok(),
            "unfenced"
        );
        assert_eq!(
            page_of("```html\n<html><body></body></html>\n```", page),
            Err("pages/home.html: CampaignList not injected".to_string())
        );
        assert_eq!(
            page_of("no page", page),
            Err("pages/home.html: the writer answered no page".to_string())
        );
    }

    #[test]
    fn a_variable_the_design_system_has_not_is_refused_with_those_it_has() {
        let tokens = ["--color-text".to_string(), "--space-1".to_string()];
        let text = "body { color: var(--color-text); background: var(--color-background); margin: var(--space-1) var(--space-6); } .g { --cols: 3; width: var(--cols); }";
        assert_eq!(
            unknown_tokens(text, &tokens),
            vec!["--color-background".to_string(), "--space-6".to_string()]
        );
        assert_eq!(
            on_tokens(text, &tokens),
            Err("it uses variables the design system has not: --color-background, --space-6; the design system's are: --color-text, --space-1".to_string())
        );
        assert!(on_tokens("a { color: var(--color-text) }", &tokens).is_ok());
        assert!(
            on_tokens(text, &[]).is_ok(),
            "no design system: nothing to check against"
        );
        assert_eq!(
            retry_prompt("ASK", "WHY"),
            "ASK\n\nYour last answer was refused: WHY. Answer again, the whole answer, with that corrected.\n"
        );
    }

    #[test]
    fn the_questions_carry_what_each_hand_needs() {
        let c = Creation {
            name: "Forge".to_string(),
            brief: "A campaign manager\nfor game masters".to_string(),
            scope: "## Goal".to_string(),
            questions: vec![super::super::creation::Question {
                id: 1,
                title: "who".to_string(),
                answer: "the GM".to_string(),
                ..Default::default()
            }],
            ..Creation::default()
        };
        let asked = plan_prompt(&c, "Card — a card");
        assert!(
            asked.contains("Card — a card")
                && asked.contains("- who — the GM")
                && asked.contains("## Goal")
        );
        assert!(plan_prompt(&c, " ").contains("`uses` stays empty"));
        let p = plan();
        let tokens = ["--color-text".to_string(), "--space-1".to_string()];
        let page = page_prompt(&c, &p, &p.pages[0], &tokens);
        assert!(page.contains("use these and no other:\n--color-text, --space-1\n"));
        assert!(
            page.contains(
                "every id a prop hands is one of these, exactly:\ncmp-1 — the campaign\n"
            )
        );
        assert!(page_prompt(&c, &p, &p.pages[0], &[]).contains("has no variable yet"));
        assert!(page.contains("The application: A campaign manager\n"));
        assert!(page.contains("- `CampaignList` — the campaigns — props: id: ID"));
        assert!(page.contains("- `campaign.html` — Campaign"));
        let alone = page_prompt(&c, &p, &p.pages[1], &tokens);
        assert!(!alone.contains("links to"));
        let index = index_html(&c, &p);
        assert!(index.contains("<title>Forge — mock-up</title>"));
        assert!(
            index.contains("<li><a href=\"pages/home.html\">Home</a> — see the campaigns</li>")
        );
    }
}
