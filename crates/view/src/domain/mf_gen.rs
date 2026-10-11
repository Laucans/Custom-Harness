//! The generation of a micro-frontend, on the design system's model: a
//! Micro-UI's spec in, a fragment of the mock-up out, written by one
//! question to a model — after a first, lighter one that plans which of the
//! kit's components it uses and which it lacks, so the missing ones are
//! generated first.
//!
//! The model never reads the kit's code. The plan reads the kit's **index**
//! — a line per component, `kit::index_line` — and the generation reads
//! the **skeletons** of the components the plan chose, `kit::skeleton`: the
//! markup's shape and classes, which the micro-frontend writes with its own
//! data while `kit.css` brings the look. The stable parts of a prompt come
//! first, the micro-frontend's own last, so a series of generations shares
//! its prefix. Pure: the questions are asked and the files written elsewhere.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::domain::capabilities::{Entry, Kind};
use crate::domain::drafts::Asked;
use harness_architecture::kit;

/// The most components a plan may ask the kit for.
pub const MAX_MISSING: usize = 3;

/// The most a fixture may weigh, in characters.
pub const MAX_FIXTURE: usize = 40_000;

/// A Micro-UI as its generation is asked for it: contract D's fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    /// `Pjs`.
    pub name: String,
    /// Its bounded context.
    #[serde(default)]
    pub system: String,
    /// What it does, in the product's words.
    #[serde(default)]
    pub description: String,
    /// The Capabilities it reads.
    #[serde(default)]
    pub needs: Vec<String>,
    /// The `DataCapabilities` it triggers.
    #[serde(default)]
    pub actions: Vec<String>,
    /// What the screen hands it.
    #[serde(default)]
    pub props: BTreeMap<String, String>,
}

impl Spec {
    /// The spec as the prompts show it, one field a line.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = format!(
            "name: {}\nsystem: {}\ndescription: {}\nneeds: {}\nactions: {}\n",
            self.name,
            self.system,
            self.description,
            self.needs.join(", "),
            self.actions.join(", ")
        );
        if !self.props.is_empty() {
            out.push_str("props:\n");
            for (key, kind) in &self.props {
                let _ = writeln!(out, "  {key}: {kind}");
            }
        }
        out
    }
}

/// A component the plan found the kit lacks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Missing {
    /// Its name for humans.
    #[serde(default)]
    pub title: String,
    /// What it shows and does, visually.
    #[serde(default)]
    pub what: String,
}

/// What the plan answers: the kit's components the micro-frontend uses,
/// and those it lacks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Names of the kit, as its index has them.
    #[serde(default)]
    pub uses: Vec<String>,
    /// What the kit has not, to generate first.
    #[serde(default)]
    pub missing: Vec<Missing>,
}

/// What the generation answers: the fragment, and the fixture when one was
/// asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    /// `mock/components/<Name>.html`.
    pub fragment: String,
    /// `mock/fixtures/<name>.json`, pretty, when the answer carried one.
    pub fixture: Option<String>,
}

/// The fragment's path, relative to `mock/`.
#[must_use]
pub fn fragment_path(name: &str) -> String {
    format!("components/{name}.html")
}

/// The draft's path, relative to `mock/`: what requests applied rewrite,
/// shown until it is saved or dropped.
#[must_use]
pub fn draft_path(name: &str) -> String {
    format!("components/{name}.draft.html")
}

/// The fixture's path, relative to `mock/`: the name in kebab case.
#[must_use]
pub fn fixture_path(name: &str) -> String {
    let mut kebab = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            kebab.push('-');
        }
        kebab.push(c.to_ascii_lowercase());
    }
    format!("fixtures/{kebab}.json")
}

/// The units of a spec as the generation reads them: a few lines each —
/// what a read is handed and returns, what a write sends and the
/// invariants it meets. A unit with no manifest yet is named alone.
#[must_use]
pub fn units_brief(spec: &Spec, entries: &[Entry]) -> String {
    let find = |id: &str| entries.iter().find(|e| e.id.eq_ignore_ascii_case(id));
    let mut out = String::new();
    for (ids, side) in [(&spec.needs, "reads"), (&spec.actions, "writes")] {
        for id in ids {
            let Some(e) = find(id) else {
                let _ = writeln!(
                    out,
                    "- {id} ({side}): no manifest yet — its name says what it does"
                );
                continue;
            };
            let k = &e.contract;
            let input = k
                .input
                .iter()
                .map(|(f, t)| format!("{f} {t}"))
                .collect::<Vec<_>>()
                .join(", ");
            match e.kind {
                Kind::Capability => {
                    let _ = writeln!(out, "- {} (reads): {}", e.id, e.role);
                    if !input.is_empty() {
                        let _ = writeln!(out, "  handed: {input}");
                    }
                    if let Some(output) = &k.output {
                        let _ = writeln!(out, "  returns: {output}");
                    }
                }
                Kind::DataCapability => {
                    let _ = writeln!(
                        out,
                        "- {} (writes: {}{}): {}",
                        e.id,
                        k.effect.as_deref().unwrap_or("write"),
                        k.target
                            .as_deref()
                            .map(|t| format!(" → {t}"))
                            .unwrap_or_default(),
                        e.role
                    );
                    if !input.is_empty() {
                        let _ = writeln!(out, "  sends: {input}");
                    }
                }
            }
            for rule in &e.rules {
                let _ = writeln!(out, "  rule {}: {}", rule.id, rule.text);
            }
        }
    }
    out
}

/// The first question: which of the kit's components the micro-frontend
/// uses, and which it lacks. The kit's index first, the spec last.
#[must_use]
pub fn plan_prompt(spec: &Spec, units: &str, index: &str) -> String {
    format!(
        "You plan one micro-frontend of a product's mock-up before it is drawn: which visual \
         components of the design system's kit it will be made of, and which it needs that the \
         kit lacks. A kit component is a visual primitive — a button, a table, a card — with no \
         data and no behaviour beyond the visual; what the micro-frontend reads or writes is its \
         own, not the kit's. Prefer the kit's components; name a missing one only when nothing \
         of the kit can show what the micro-frontend must show, {MAX_MISSING} at most.\n\n\
         Answer ONLY a JSON object, no prose, no code fence: \
         {{\"uses\": [\"<a name of the kit>\", …], \"missing\": [{{\"title\": \"<a name for \
         humans>\", \"what\": \"<what it shows and does, visually, one sentence>\"}}, …]}}.\n\n\
         The kit, one component a line:\n{index}\n\
         The micro-frontend:\n{spec}\n\
         What it reads and writes:\n{units}",
        index = if index.trim().is_empty() {
            "(the kit is empty)\n"
        } else {
            index
        },
        spec = spec.text(),
        units = if units.trim().is_empty() {
            "nothing\n"
        } else {
            units
        },
    )
}

/// The plan an answer holds: the first JSON object in it.
///
/// # Errors
///
/// No JSON object, or not a plan.
pub fn plan_of(answer: &str) -> Result<Plan, String> {
    let start = answer.find('{').ok_or("the plan is not JSON")?;
    let end = answer.rfind('}').ok_or("the plan is not JSON")?;
    let text = answer.get(start..=end).ok_or("the plan is not JSON")?;
    let mut plan: Plan = serde_json::from_str(text).map_err(|e| format!("the plan: {e}"))?;
    plan.missing.retain(|m| !m.title.trim().is_empty());
    plan.missing.truncate(MAX_MISSING);
    plan.uses.retain(|u| kit::is_name(u));
    Ok(plan)
}

/// The second question: the micro-frontend itself. Its rules and the kit's
/// skeletons first, the spec, its units and its fixture last.
#[must_use]
pub fn generate_prompt(
    spec: &Spec,
    units: &str,
    skeletons: &str,
    tokens: &str,
    fixture: Option<&str>,
) -> String {
    let fixture_at = fixture_path(&spec.name);
    let mut prompt = String::from(
        "You write one micro-frontend of a product's mock-up: a fragment of HTML a static page \
         injects in `<div data-component=\"<Name>\" data-props='{…}'>`, shown on a fixture — the \
         data its Capabilities would answer — with no backend ever.\n\n\
         The rules:\n\
         - Answer ONLY a ```html block (the fragment), then — when asked below — a ```json block \
         (the fixture). No prose.\n\
         - The fragment is its markup, then one <script> in an arrow IIFE: `const root = \
         document.currentScript.closest('[data-component]'); const props = \
         JSON.parse(root.dataset.props || '{}');`. It reads its data from one fetch of the fixture, \
         `fetch(new URL('../<fixture>', location.href))` — no other network, no external script or \
         stylesheet — and filters it by the props. Shown alone it is handed no props: a prop \
         missing falls back to the fixture's first fitting value, so it always shows its data.\n\
         - It opens at rest: what overlays — a dialog, a toast, a drop list — stays hidden \
         until its actuator opens it, and closes again.\n\
         - Every actuator carries `data-writes=\"<DataCapability>\"`, every part that shows a read \
         `data-reads=\"<Capability>\"`: each action and each need of the spec at least once.\n\
         - A write calls no server: it checks the rules below in the page, shows a violation next \
         to its field (`aria-invalid`, a short message) and refuses; else it shows the command's \
         life — queued, then applied — and updates what is shown.\n\
         - The look is the design system's: write the kit's components with the markup and the \
         classes of their skeletons below, your own content in place of the ellipses (`kit.css` \
         styles them). Any other style only lays out — through `var(--…)` of the tokens below, \
         never a colour literal.\n\
         - The words on screen in the language of the description.\n\n",
    );
    let _ = write!(
        prompt,
        "The kit's components it uses, as skeletons:\n{}\nThe tokens (variables):\n{}\n\n",
        if skeletons.trim().is_empty() {
            "(none: plain markup)"
        } else {
            skeletons
        },
        tokens
    );
    let _ = write!(
        prompt,
        "The micro-frontend:\n{}\nWhat it reads and writes:\n{}\n",
        spec.text(),
        units
    );
    match fixture {
        Some(text) => {
            let _ = write!(
                prompt,
                "Its fixture, `{fixture_at}`, exists — read it, do not answer one:\n{text}\n"
            );
        }
        None => {
            let _ = writeln!(
                prompt,
                "It has no fixture yet: answer one after the fragment, the ```json of `{fixture_at}` \
                 — a few rows shaped as its reads return them, enough to show every state."
            );
        }
    }
    prompt
}

/// The question that applies requests to a micro-frontend: its rules, the
/// kit's index and the tokens first, then the micro-frontend, its units, its
/// fragment as the draft has it, and the requests — one on an element names
/// it by its path of children from the fragment's root.
#[must_use]
pub fn rewrite_prompt(
    spec: &Spec,
    units: &str,
    index: &str,
    tokens: &str,
    fragment: &str,
    asked: &[Asked],
) -> String {
    let requests = asked
        .iter()
        .map(|a| {
            if a.target.trim().is_empty() {
                format!("- request `{}`: {}", a.id, a.text.trim())
            } else {
                format!(
                    "- request `{}`, on the element `{}`: {}",
                    a.id,
                    a.target.trim(),
                    a.text.trim()
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You edit one micro-frontend of a product's mock-up on behalf of a human who dropped \
         requests on it. Apply every request to the fragment below and answer the WHOLE fragment \
         back in a ```html block — its markup and its script — no prose. Keep what the requests do \
         not touch as it is, and keep its rules: its data from the one fetch of its fixture, no \
         other network; `data-writes` on every actuator and `data-reads` on every part that shows \
         a read; a write checks the rules below in the page and shows the command's life; it opens \
         at rest; the kit's components written with their classes, any other style through \
         `var(--…)`, never a colour literal. A request that needs data or a unit the \
         micro-frontend does not have is not a mock-up's matter: leave the fragment unchanged for \
         it and say so after the fragment, inside it, as `<!-- skipped <id>: <why> -->`.\n\n\
         The kit, one component a line:\n{index}\nThe tokens (variables):\n{tokens}\n\n\
         The micro-frontend:\n{spec}\nWhat it reads and writes:\n{units}\n\
         The requests:\n{requests}\n\n\
         The fragment:\n<fragment>\n{fragment}\n</fragment>\n",
        index = if index.trim().is_empty() {
            "(the kit is empty)\n"
        } else {
            index
        },
        spec = spec.text(),
        units = if units.trim().is_empty() {
            "nothing\n"
        } else {
            units
        },
        fragment = fragment.trim(),
    )
}

/// `mock/pages.json` with a fork of a micro-frontend listed: the origin's
/// entry under a new name and fragment, everything else as it was.
///
/// # Errors
///
/// The manifest is not JSON, has no such micro-frontend, or has the name
/// taken already.
pub fn forked(pages: &str, from: &str, to: &str) -> Result<String, String> {
    let mut root: Value = serde_json::from_str(pages).map_err(|e| format!("pages.json: {e}"))?;
    let list = root
        .get_mut("components")
        .and_then(Value::as_array_mut)
        .ok_or("pages.json lists no micro-frontend")?;
    if list
        .iter()
        .any(|c| c.get("name").and_then(Value::as_str) == Some(to))
    {
        return Err(format!("{to} is a micro-frontend already"));
    }
    let mut entry = list
        .iter()
        .find(|c| c.get("name").and_then(Value::as_str) == Some(from))
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| format!("pages.json does not list {from}"))?;
    entry.insert("name".into(), Value::String(to.to_string()));
    entry.insert("fragment".into(), Value::String(fragment_path(to)));
    let described = entry
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    entry.insert(
        "description".into(),
        Value::String(format!("{described} (a fork of {from})").trim().to_string()),
    );
    entry.remove("source");
    entry.remove("builtFrom");
    list.push(Value::Object(entry));
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?
    ))
}

/// The name a fork takes when the human gives none: the origin's, with the
/// first number that is free.
#[must_use]
pub fn fork_name(pages: &str, from: &str) -> String {
    let taken = |name: &str| pages.contains(&format!("\"{name}\""));
    (2..1000)
        .map(|n| format!("{from}{n}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| format!("{from}Fork"))
}

/// The fragment and the fixture an answer carries.
///
/// # Errors
///
/// No HTML block or no markup, a fragment too heavy, a fixture not JSON
/// or too heavy.
pub fn generated_of(answer: &str) -> Result<Generated, String> {
    let fragment = block(answer, "html")
        .or_else(|| (!answer.contains("```")).then(|| answer.trim().to_string()))
        .ok_or("the answer carries no ```html block")?;
    if !fragment.contains('<') || !fragment.contains("data-") {
        return Err("the fragment carries no markup".to_string());
    }
    if fragment.chars().count() > kit::MAX_FRAGMENT {
        return Err(format!(
            "the fragment weighs more than {} characters",
            kit::MAX_FRAGMENT
        ));
    }
    let fixture = match block(answer, "json") {
        Some(text) => {
            if text.chars().count() > MAX_FIXTURE {
                return Err(format!(
                    "the fixture weighs more than {MAX_FIXTURE} characters"
                ));
            }
            let value: Value =
                serde_json::from_str(&text).map_err(|e| format!("the fixture: {e}"))?;
            Some(format!(
                "{}\n",
                serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
            ))
        }
        None => None,
    };
    Ok(Generated {
        fragment: format!("{}\n", fragment.trim()),
        fixture,
    })
}

/// The body of the first fenced block of a language in a text.
fn block(text: &str, lang: &str) -> Option<String> {
    let open = format!("```{lang}");
    let at = text.find(&open)?;
    let after = text.get(at.saturating_add(open.len())..)?;
    let body = after.split_once('\n').map_or("", |(_, b)| b);
    let end = body.find("```")?;
    Some(body.get(..end)?.trim().to_string())
}

/// What the parts named in a fragment leave out of the spec: the actions
/// and needs no `data-writes`/`data-reads` names.
#[must_use]
pub fn untagged(fragment: &str, spec: &Spec) -> Vec<String> {
    let lower = fragment.to_lowercase();
    let named = |attr: &str, id: &str| {
        let id = id.to_lowercase();
        lower.match_indices(attr).any(|(at, _)| {
            lower
                .get(at..at.saturating_add(attr.len() + 200).min(lower.len()))
                .is_some_and(|near| near.contains(&id))
        })
    };
    spec.actions
        .iter()
        .filter(|a| !named("data-writes", a))
        .chain(spec.needs.iter().filter(|n| !named("data-reads", n)))
        .cloned()
        .collect()
}

/// `mock/pages.json` with the micro-frontend listed as generated: its
/// entry made or brought up to date, everything else as it was.
///
/// # Errors
///
/// The manifest is not JSON, or not an object.
pub fn listed(pages: Option<&str>, spec: &Spec, with_fixture: bool) -> Result<String, String> {
    let mut root: Value = match pages {
        Some(text) if !text.trim().is_empty() => {
            serde_json::from_str(text).map_err(|e| format!("pages.json: {e}"))?
        }
        _ => serde_json::json!({ "version": 1, "pages": [], "components": [] }),
    };
    let object = root.as_object_mut().ok_or("pages.json is not an object")?;
    let components = object
        .entry("components")
        .or_insert_with(|| Value::Array(Vec::new()));
    let list = components
        .as_array_mut()
        .ok_or("pages.json: components is not a list")?;
    let at = list
        .iter()
        .position(|c| c.get("name").and_then(Value::as_str) == Some(spec.name.as_str()));
    let mut entry = at
        .and_then(|i| list.get(i))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);
    entry.insert("name".into(), Value::String(spec.name.clone()));
    entry.insert("system".into(), Value::String(spec.system.clone()));
    entry.insert(
        "description".into(),
        Value::String(spec.description.clone()),
    );
    entry.insert("fragment".into(), Value::String(fragment_path(&spec.name)));
    if with_fixture || entry.contains_key("fixture") {
        entry.insert("fixture".into(), Value::String(fixture_path(&spec.name)));
    }
    entry.insert("needs".into(), serde_json::json!(spec.needs));
    entry.insert("actions".into(), serde_json::json!(spec.actions));
    entry.insert("props".into(), serde_json::json!(spec.props));
    match at {
        Some(i) => {
            if let Some(slot) = list.get_mut(i) {
                *slot = Value::Object(entry);
            }
        }
        None => list.push(Value::Object(entry)),
    }
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::capabilities::{Contract, Rule, Status};

    fn spec() -> Spec {
        Spec {
            name: "Pjs".into(),
            system: "campagne".into(),
            description: "Liste et ajoute les PJ".into(),
            needs: vec!["campagne.listerPjs".into()],
            actions: vec!["campagne.ajouterPj".into()],
            props: BTreeMap::from([("campagneId".to_string(), "ID".to_string())]),
        }
    }

    fn entry(id: &str, kind: Kind) -> Entry {
        Entry {
            id: id.into(),
            kind,
            system: "campagne".into(),
            name: id.split('.').next_back().unwrap_or_default().into(),
            status: Status::Available,
            created: None,
            modified: None,
            role: "does it".into(),
            path: None,
            version: None,
            branches: vec![],
            contract: Contract {
                input: BTreeMap::from([("nom".to_string(), "string".to_string())]),
                output: (kind == Kind::Capability).then(|| "[{ id: ID }]".to_string()),
                effect: (kind == Kind::DataCapability).then(|| "insert".to_string()),
                target: (kind == Kind::DataCapability).then(|| "PJ".to_string()),
                ..Contract::default()
            },
            used_by: vec![],
            rules: vec![Rule {
                id: "pc-name-required".into(),
                text: "A name.".into(),
                from: "aggregate PJ".into(),
            }],
            issues: vec![],
        }
    }

    #[test]
    fn the_units_are_briefed_with_their_contract_and_rules() {
        let brief = units_brief(
            &spec(),
            &[
                entry("campagne.listerPjs", Kind::Capability),
                entry("campagne.ajouterpj", Kind::DataCapability),
            ],
        );
        assert!(brief.contains("- campagne.listerPjs (reads): does it\n  handed: nom string\n  returns: [{ id: ID }]"), "{brief}");
        assert!(
            brief.contains("(writes: insert → PJ)")
                && brief.contains("  sends: nom string")
                && brief.contains("rule pc-name-required: A name."),
            "{brief}"
        );
        let unknown = units_brief(&spec(), &[]);
        assert!(unknown.contains("campagne.ajouterPj (writes): no manifest yet"));
    }

    #[test]
    fn a_plan_is_read_off_its_json_and_kept_within_bounds() {
        let plan = plan_of("here:\n{\"uses\": [\"Table\", \"not a name\"], \"missing\": [{\"title\": \"Fiche\", \"what\": \"a sheet\"}, {\"title\": \"\"}, {\"title\": \"A\"}, {\"title\": \"B\"}, {\"title\": \"C\"}]}").expect("plan");
        assert_eq!(plan.uses, ["Table"]);
        assert_eq!(plan.missing.len(), MAX_MISSING);
        assert_eq!(plan.missing[0].title, "Fiche");
        assert!(plan_of("no").is_err());
        let prompt = plan_prompt(
            &spec(),
            "- units\n",
            "- Table (Table): rows — kit-table: num\n",
        );
        assert!(
            prompt.find("The kit, one component a line").unwrap_or(0)
                < prompt.find("The micro-frontend:").unwrap_or(0)
        );
    }

    #[test]
    fn the_generation_reads_skeletons_and_answers_a_fragment_and_its_fixture() {
        let prompt = generate_prompt(
            &spec(),
            "- units\n",
            "Table: <table class=\"kit-table\">…</table>",
            "--color-primary",
            None,
        );
        assert!(
            prompt.contains("data-writes=")
                && prompt.contains("fixtures/pjs.json")
                && prompt.contains("```json")
        );
        assert!(
            prompt.find("skeletons").unwrap_or(usize::MAX)
                < prompt.find("The micro-frontend:\n").unwrap_or(0),
            "the stable part first"
        );
        let with = generate_prompt(&spec(), "", "", "", Some("[]"));
        assert!(with.contains("exists — read it"));
        let got = generated_of("```html\n<table data-reads=\"campagne.listerPjs\"></table>\n```\n```json\n[{\"id\":1}]\n```").expect("generated");
        assert!(got.fragment.starts_with("<table"));
        assert_eq!(
            got.fixture.as_deref(),
            Some("[\n  {\n    \"id\": 1\n  }\n]\n")
        );
        assert!(generated_of("```json\n[]\n```").is_err());
        assert!(generated_of("```html\n<div data-x></div>\n```\n```json\nnot json\n```").is_err());
        assert_eq!(
            untagged("<button data-writes=\"campagne.ajouterPj\">", &spec()),
            ["campagne.listerPjs"]
        );
    }

    #[test]
    fn a_rewrite_names_the_parts_and_a_fork_is_listed_beside_its_origin() {
        let prompt = rewrite_prompt(
            &spec(),
            "- units\n",
            "- Table (Table): rows\n",
            "--color-primary",
            "<ul data-reads=\"campagne.listerPjs\"></ul>",
            &[Asked {
                id: "r1".into(),
                text: "bolder".into(),
                target: "ul:nth-child(1)".into(),
            }],
        );
        assert!(
            prompt.contains("- request `r1`, on the element `ul:nth-child(1)`: bolder"),
            "{prompt}"
        );
        assert!(
            prompt
                .find("The kit, one component a line")
                .unwrap_or(usize::MAX)
                < prompt.find("<fragment>").unwrap_or(0)
        );
        let pages = r#"{"version":1,"pages":[],"components":[{"name":"Pjs","fragment":"components/Pjs.html","description":"Les PJ","source":"src/Pjs.js"}]}"#;
        assert_eq!(fork_name(pages, "Pjs"), "Pjs2");
        let after: Value =
            serde_json::from_str(&forked(pages, "Pjs", "Pjs2").expect("forked")).expect("json");
        assert_eq!(after["components"][1]["fragment"], "components/Pjs2.html");
        assert_eq!(
            after["components"][1]["description"],
            "Les PJ (a fork of Pjs)"
        );
        assert!(
            after["components"][1].get("source").is_none(),
            "a fork is hand-written"
        );
        assert!(forked(pages, "Pjs", "Pjs").is_err());
        assert!(forked(pages, "None", "X").is_err());
        assert_eq!(draft_path("Pjs"), "components/Pjs.draft.html");
    }

    #[test]
    fn the_manifest_lists_the_micro_frontend_and_keeps_the_rest() {
        let before = r#"{"version":1,"pages":[{"path":"pages/a.html"}],"components":[{"name":"Pjs","fixture":"fixtures/old.json","extra":true},{"name":"Other"}]}"#;
        let after: Value =
            serde_json::from_str(&listed(Some(before), &spec(), false).expect("listed"))
                .expect("json");
        let pjs = &after["components"][0];
        assert_eq!(pjs["fragment"], "components/Pjs.html");
        assert_eq!(pjs["fixture"], "fixtures/pjs.json");
        assert_eq!(pjs["extra"], true);
        assert_eq!(after["components"][1]["name"], "Other");
        assert_eq!(after["pages"][0]["path"], "pages/a.html");
        let fresh: Value =
            serde_json::from_str(&listed(None, &spec(), true).expect("new")).expect("json");
        assert_eq!(fresh["components"][0]["actions"][0], "campagne.ajouterPj");
        assert_eq!(
            fixture_path("NiveauDuGroupe"),
            "fixtures/niveau-du-groupe.json"
        );
    }
}
