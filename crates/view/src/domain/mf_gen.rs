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

/// What the generation answers: the fragment, and the fixtures it was
/// asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    /// `mock/components/<Name>.html`.
    pub fragment: String,
    /// Each fixture asked for, by Capability: `mock/fixtures/<Capability>.json`,
    /// pretty.
    pub fixtures: Vec<(String, String)>,
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

/// The fixture of one Capability, relative to `mock/`: what the mock server
/// answers it with (`fixtures/<identifier>.json`), what a micro-frontend
/// that needs it reads, what the rules' tests start from.
#[must_use]
pub fn fixture_path(capability: &str) -> String {
    format!("fixtures/{capability}.json")
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
    fixtures: &[(String, Option<String>)],
) -> String {
    let mut prompt = String::from(
        "You write one micro-frontend of a product's mock-up: a fragment of HTML a static page \
         injects in `<div data-component=\"<Name>\" data-props='{…}'>`, shown on a fixture — the \
         data its Capabilities would answer — with no backend ever.\n\n\
         The rules:\n\
         - Answer ONLY a ```html block (the fragment), then — when asked below — a ```json block \
         (the fixtures). No prose.\n\
         - The fragment is its markup, then one <script> in an arrow IIFE: `const root = \
         document.currentScript.closest('[data-component]'); const props = \
         JSON.parse(root.dataset.props || '{}');`. It reads its data with one fetch per Capability \
         it needs, `fetch(new URL('../fixtures/<Capability>.json', location.href))` — what the \
         mock server answers that Capability with — no other network, no external script or \
         stylesheet — and filters it by the props. Shown alone it is handed no props: a prop \
         missing falls back to the fixtures' first fitting value, so it always shows its data.\n\
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
    let mut missing = Vec::new();
    for (capability, text) in fixtures {
        match text {
            Some(text) => {
                let _ = write!(
                    prompt,
                    "The fixture of `{capability}`, `{}`, exists — read it, do not answer it:\n{text}\n",
                    fixture_path(capability)
                );
            }
            None => missing.push(format!("`{capability}`")),
        }
    }
    if !missing.is_empty() {
        let _ = writeln!(
            prompt,
            "No fixture yet for {}: answer them after the fragment, in one ```json block holding \
             an object keyed by Capability — `{{\"<Capability>\": <what it answers>}}` — a few \
             rows each, shaped as the reads return them, enough to show every state.",
            missing.join(", ")
        );
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
         not touch as it is, and keep its rules: its data from one fetch per Capability it needs \
         (`fixtures/<Capability>.json`), no other network; `data-writes` on every actuator and `data-reads` on every part that shows \
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

/// `mock/pages.json` with one page injecting `to` where it injected `from`:
/// a fork made for that page alone, the other pages as they were.
///
/// # Errors
///
/// The manifest is not JSON, or lists no such page.
pub fn page_forked(pages: &str, page: &str, from: &str, to: &str) -> Result<String, String> {
    let mut root: Value = serde_json::from_str(pages).map_err(|e| format!("pages.json: {e}"))?;
    let entry = root
        .get_mut("pages")
        .and_then(Value::as_array_mut)
        .and_then(|list| {
            list.iter_mut()
                .find(|p| p.get("path").and_then(Value::as_str) == Some(page))
        })
        .ok_or_else(|| format!("pages.json does not list {page}"))?;
    if let Some(list) = entry.get_mut("components").and_then(Value::as_array_mut) {
        for name in list.iter_mut() {
            if name.as_str() == Some(from) {
                *name = Value::String(to.to_string());
            }
        }
    }
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?
    ))
}

/// A page's markup injecting `to` where it injected `from`: every
/// `data-component` naming it, quoted either way.
#[must_use]
pub fn retargeted(html: &str, from: &str, to: &str) -> String {
    html.replace(
        &format!("data-component=\"{from}\""),
        &format!("data-component=\"{to}\""),
    )
    .replace(
        &format!("data-component='{from}'"),
        &format!("data-component='{to}'"),
    )
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

/// The fragment and the fixtures an answer carries: one per Capability of
/// `missing`, read off the fenced `json` object keyed by them.
///
/// # Errors
///
/// No HTML block or no markup, a fragment too heavy, fixtures not JSON, too
/// heavy, or lacking one of `missing`.
pub fn generated_of(answer: &str, missing: &[String]) -> Result<Generated, String> {
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
    let mut fixtures = Vec::new();
    if !missing.is_empty() {
        let text =
            block(answer, "json").ok_or("the answer carries no ```json block of fixtures")?;
        if text.chars().count() > MAX_FIXTURE {
            return Err(format!(
                "the fixtures weigh more than {MAX_FIXTURE} characters"
            ));
        }
        let value: Value = serde_json::from_str(&text).map_err(|e| format!("the fixtures: {e}"))?;
        for capability in missing {
            let one = value
                .get(capability)
                .ok_or_else(|| format!("the fixtures lack `{capability}`"))?;
            fixtures.push((
                capability.clone(),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(one).map_err(|e| e.to_string())?
                ),
            ));
        }
    }
    Ok(Generated {
        fragment: format!("{}\n", fragment.trim()),
        fixtures,
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
    unnamed(fragment, &spec.needs, &spec.actions)
}

/// The actions no `data-writes` and the needs no `data-reads` of a fragment
/// names (contract D), actions first.
#[must_use]
pub fn unnamed(fragment: &str, needs: &[String], actions: &[String]) -> Vec<String> {
    let lower = fragment.to_lowercase();
    let named = |attr: &str, id: &str| {
        let id = id.to_lowercase();
        lower.match_indices(attr).any(|(at, _)| {
            lower
                .get(at..at.saturating_add(attr.len() + 200).min(lower.len()))
                .is_some_and(|near| near.contains(&id))
        })
    };
    actions
        .iter()
        .filter(|a| !named("data-writes", a))
        .chain(needs.iter().filter(|n| !named("data-reads", n)))
        .cloned()
        .collect()
}

/// `mock/pages.json` with the micro-frontend listed as generated: its
/// entry made or brought up to date, everything else as it was.
///
/// # Errors
///
/// The manifest is not JSON, or not an object.
pub fn listed(pages: Option<&str>, spec: &Spec, fixtures: &[String]) -> Result<String, String> {
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
    // One fixture per Capability it needs; the old single file goes.
    entry.remove("fixture");
    if fixtures.is_empty() {
        entry.remove("fixtures");
    } else {
        entry.insert("fixtures".into(), serde_json::json!(fixtures));
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
            &[("campagne.listerPjs".to_string(), None)],
        );
        assert!(
            prompt.contains("data-writes=")
                && prompt.contains("fixtures/<Capability>.json")
                && prompt.contains("No fixture yet for `campagne.listerPjs`")
                && prompt.contains("```json")
        );
        assert!(
            prompt.find("skeletons").unwrap_or(usize::MAX)
                < prompt.find("The micro-frontend:\n").unwrap_or(0),
            "the stable part first"
        );
        let with = generate_prompt(
            &spec(),
            "",
            "",
            "",
            &[("campagne.listerPjs".to_string(), Some("[]".to_string()))],
        );
        assert!(with.contains("`fixtures/campagne.listerPjs.json`, exists — read it"));
        assert!(!with.contains("No fixture yet"));
        let missing = ["campagne.listerPjs".to_string()];
        let got = generated_of(
            "```html\n<table data-reads=\"campagne.listerPjs\"></table>\n```\n```json\n{\"campagne.listerPjs\": [{\"id\":1}], \"other\": 2}\n```",
            &missing,
        )
        .expect("generated");
        assert!(got.fragment.starts_with("<table"));
        assert_eq!(
            got.fixtures,
            [(
                "campagne.listerPjs".to_string(),
                "[\n  {\n    \"id\": 1\n  }\n]\n".to_string()
            )]
        );
        let none = generated_of(
            "```html\n<div data-x></div>\n```\n```json\nnot json\n```",
            &[],
        )
        .expect("nothing asked, nothing read");
        assert!(none.fixtures.is_empty());
        assert!(generated_of("```json\n[]\n```", &missing).is_err());
        assert!(
            generated_of(
                "```html\n<div data-x></div>\n```\n```json\nnot json\n```",
                &missing
            )
            .is_err()
        );
        assert!(
            generated_of(
                "```html\n<div data-x></div>\n```\n```json\n{\"x\": 1}\n```",
                &missing
            )
            .expect_err("lacks it")
            .contains("lack `campagne.listerPjs`")
        );
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
                ..Asked::default()
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
    fn a_fork_for_one_page_is_injected_there_alone() {
        let pages = r#"{"pages":[{"path":"pages/a.html","components":["Pjs","Map"]},{"path":"pages/b.html","components":["Pjs"]}],"components":[]}"#;
        let after: Value = serde_json::from_str(
            &page_forked(pages, "pages/a.html", "Pjs", "Pjs2").expect("forked"),
        )
        .expect("json");
        assert_eq!(
            after["pages"][0]["components"],
            serde_json::json!(["Pjs2", "Map"])
        );
        assert_eq!(after["pages"][1]["components"], serde_json::json!(["Pjs"]));
        assert!(page_forked(pages, "pages/c.html", "Pjs", "Pjs2").is_err());
        assert!(page_forked("not json", "pages/a.html", "Pjs", "Pjs2").is_err());
        let html = r#"<div data-component="Pjs"></div><p data-component='Pjs'></p><i data-component="Pjs3"></i>"#;
        assert_eq!(
            retargeted(html, "Pjs", "Pjs2"),
            r#"<div data-component="Pjs2"></div><p data-component='Pjs2'></p><i data-component="Pjs3"></i>"#
        );
    }

    #[test]
    fn the_manifest_lists_the_micro_frontend_and_keeps_the_rest() {
        let before = r#"{"version":1,"pages":[{"path":"pages/a.html"}],"components":[{"name":"Pjs","fixture":"fixtures/old.json","extra":true},{"name":"Other"}]}"#;
        let paths = [fixture_path("campagne.listerPjs")];
        let after: Value =
            serde_json::from_str(&listed(Some(before), &spec(), &paths).expect("listed"))
                .expect("json");
        let pjs = &after["components"][0];
        assert_eq!(pjs["fragment"], "components/Pjs.html");
        assert!(pjs.get("fixture").is_none(), "the old single file goes");
        assert_eq!(
            pjs["fixtures"],
            serde_json::json!(["fixtures/campagne.listerPjs.json"])
        );
        assert_eq!(pjs["extra"], true);
        assert_eq!(after["components"][1]["name"], "Other");
        assert_eq!(after["pages"][0]["path"], "pages/a.html");
        let fresh: Value =
            serde_json::from_str(&listed(None, &spec(), &[]).expect("new")).expect("json");
        assert!(fresh["components"][0].get("fixtures").is_none());
        assert_eq!(fresh["components"][0]["actions"][0], "campagne.ajouterPj");
        assert_eq!(
            fixture_path("groupe.lireNiveau"),
            "fixtures/groupe.lireNiveau.json"
        );
    }
}
