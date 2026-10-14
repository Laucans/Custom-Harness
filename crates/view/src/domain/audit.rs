//! The architecture's guard on a creation's rules step: the cut of the
//! mock-up into micro-frontends, Capabilities and `DataCapabilities`,
//! checked against the vertical architecture before the factory builds it.
//!
//! Three checks are read off the cut itself — the names, the systems'
//! sizes, a micro-frontend reaching into another system. Three need a
//! reading of the meaning, and an AI makes it — two units for one thing, a
//! write passed off as a read, a Capability leaning on another — along with
//! the effect of every write, which says whether its task may run beside
//! the others. Only a write passed off as a read blocks: it breaks the
//! frontier between reading and writing. Pure: the asking is the desk's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::creation::{ComponentRef, Context};

/// A system with more micro-frontends than this is to split.
pub const SYSTEM_MAX: usize = 8;

/// What a finding is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// Two units that mean the same thing under two names.
    Duplicate,
    /// A unit not named `<system>.<verbThing>`.
    Naming,
    /// A system too big to stay one bounded context, or too thin to be one.
    SystemSize,
    /// A micro-frontend reading or writing another system's units.
    MixedSystems,
    /// A write passed off as a read: a Capability that changes data.
    WriteAsRead,
    /// A Capability that leans on another one.
    CapabilityCalls,
    /// A micro-frontend whose fragment names no part for one of its needs
    /// or actions (contract D: `data-reads`, `data-writes`).
    PartUnnamed,
}

impl Check {
    /// It keeps the rules from being validated.
    #[must_use]
    pub const fn blocks(self) -> bool {
        matches!(self, Self::WriteAsRead | Self::PartUnnamed)
    }
}

/// What a write does to the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// A new row only: its task may run beside the others (the page says
    /// so — every other effect runs alone).
    Insert,
    /// Changes a row that exists.
    Update,
    /// Removes a row.
    Delete,
    /// Adds or changes.
    Upsert,
}

/// One thing the guard found, and what it proposes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Finding {
    /// Its number in the audit.
    pub id: u32,
    /// What it is about; `None` when the AI named a check it does not have.
    pub check: Option<Check>,
    /// The units it is about: micro-frontends, Capabilities,
    /// `DataCapabilities`, systems.
    pub units: Vec<String>,
    /// What is wrong, in the language of the brief.
    pub why: String,
    /// What the guard proposes to do about it.
    pub fix: String,
}

impl Finding {
    /// It keeps the rules from being validated.
    #[must_use]
    pub fn blocks(&self) -> bool {
        self.check.is_some_and(Check::blocks)
    }
}

/// What the guard found on a cut.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audit {
    /// The findings left: applied or dismissed ones are gone.
    pub findings: Vec<Finding>,
    /// Every write's effect, by `DataCapability`.
    pub effects: BTreeMap<String, Effect>,
    /// The cut it was made on: `componentized` as it was said then.
    pub from: String,
}

impl Audit {
    /// The findings that keep the rules from being validated.
    #[must_use]
    pub fn blocking(&self) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.blocks()).collect()
    }

    /// A finding taken away — applied, or dismissed by the human.
    ///
    /// # Errors
    ///
    /// No finding has that number, or a blocking one is dismissed: only
    /// its fix takes it away.
    pub fn resolve(&mut self, id: u32, applied: bool) -> Result<Finding, String> {
        let at = self
            .findings
            .iter()
            .position(|f| f.id == id)
            .ok_or_else(|| format!("no finding {id}"))?;
        if !applied && self.findings[at].blocks() {
            return Err(match self.findings[at].check {
                Some(Check::PartUnnamed) => {
                    "a part a micro-frontend does not name is named, not dismissed".to_string()
                }
                _ => "a write passed off as a read is fixed, not dismissed".to_string(),
            });
        }
        Ok(self.findings.remove(at))
    }
}

/// The micro-frontends whose fragment, as read, names no part for one of
/// its needs or actions: what a part must carry for the gates and the
/// rules' tests to find it (contract D). A fragment not read is not judged.
#[must_use]
pub fn unnamed_parts(components: &[ComponentRef]) -> Vec<Finding> {
    components
        .iter()
        .filter(|c| !c.markup.trim().is_empty())
        .filter_map(|c| {
            let unnamed = super::mf_gen::unnamed(&c.markup, &c.needs, &c.actions);
            (!unnamed.is_empty()).then(|| Finding {
                id: 0,
                check: Some(Check::PartUnnamed),
                units: std::iter::once(c.name.clone())
                    .chain(unnamed.iter().cloned())
                    .collect(),
                why: format!(
                    "{} names no part for {}: nothing a gate or a rule's test can find",
                    c.name,
                    unnamed.join(", ")
                ),
                fix: format!(
                    "in `mock/{}`, mark the part that shows each read `data-reads=\"<Capability>\"` \
                     and the actuator of each write `data-writes=\"<DataCapability>\"`: {}",
                    c.fragment,
                    unnamed.join(", ")
                ),
            })
        })
        .collect()
}

/// The checks read off the cut alone: the names, the systems' sizes, the
/// micro-frontends reaching into another system, the parts a fragment does
/// not name.
#[must_use]
pub fn local_findings(components: &[ComponentRef]) -> Vec<Finding> {
    let mut found = unnamed_parts(components);
    let units: BTreeSet<&str> = components
        .iter()
        .flat_map(|c| c.needs.iter().chain(&c.actions))
        .map(String::as_str)
        .collect();
    for unit in units.iter().filter(|u| !well_named(u)) {
        found.push(Finding {
            check: Some(Check::Naming),
            units: vec![(*unit).to_string()],
            why: format!("`{unit}` is not named `<system>.<verbThing>`"),
            fix: format!("rename `{unit}` as `<system>.<verbThing>` wherever it is listed"),
            ..Finding::default()
        });
    }
    let mut sizes: BTreeMap<&str, usize> = BTreeMap::new();
    for c in components {
        *sizes.entry(c.system.as_str()).or_default() += 1;
    }
    for (system, count) in &sizes {
        if *count > SYSTEM_MAX {
            found.push(Finding {
                check: Some(Check::SystemSize),
                units: vec![(*system).to_string()],
                why: format!(
                    "`{system}` holds {count} micro-frontends: more than one bounded context"
                ),
                fix: format!("split `{system}` into the bounded contexts it holds"),
                ..Finding::default()
            });
        } else if *count == 1 && sizes.len() > 1 {
            found.push(Finding {
                check: Some(Check::SystemSize),
                units: vec![(*system).to_string()],
                why: format!("`{system}` holds one micro-frontend only"),
                fix: format!("keep `{system}` if it is a context of its own, else fold it into the system it serves"),
                ..Finding::default()
            });
        }
    }
    for c in components {
        let foreign: BTreeSet<&str> = c
            .needs
            .iter()
            .chain(&c.actions)
            .filter_map(|u| u.split_once('.').map(|(system, _)| system))
            .filter(|system| *system != c.system)
            .collect();
        if !foreign.is_empty() {
            let systems: Vec<&str> = foreign.into_iter().collect();
            found.push(Finding {
                check: Some(Check::MixedSystems),
                units: vec![c.name.clone()],
                why: format!(
                    "`{}` ({}) reaches into {}",
                    c.name,
                    c.system,
                    systems.join(", ")
                ),
                fix: format!(
                    "move `{}` to the system it serves, or give `{}` the units it needs",
                    c.name, c.system
                ),
                ..Finding::default()
            });
        }
    }
    found
}

/// `<system>.<verbThing>`: a lowercase system, a dot, a camelCase name.
fn well_named(unit: &str) -> bool {
    let Some((system, name)) = unit.split_once('.') else {
        return false;
    };
    let lower = |s: &str| s.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    lower(system)
        && system
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && lower(name)
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The question the guard asks: the architecture's rules, the cut, and the
/// three checks only a reading of the meaning makes, with every write's
/// effect.
#[must_use]
pub fn audit_prompt(ctx: &Context, language: &str) -> String {
    let mut text = String::from(
        "You guard a vertical-slice architecture before agents build it. Its rules: a \
         Micro-UI (micro-frontend) declares the Capabilities it reads (`needs`) and the \
         DataCapabilities it triggers (`actions`). A Capability is strictly read-only: it \
         collects, transforms, enriches, never changes data, and it never invokes, imports or \
         references another Capability — what it needs of another it implements locally. Only a \
         DataCapability writes; its effect is `insert` (a new row only: its task runs beside the \
         others), `update`, `delete` or `upsert` (its task runs alone). Units are named \
         `<system>.<verbThing>`; a system is one bounded context.\n\n\
         Read the cut below and report, as findings, only these three things:\n\
         - `duplicate`: two or more units (or micro-frontends) that mean the same thing under \
         different names;\n\
         - `write_as_read`: a unit listed under `needs` whose name or role changes data (it \
         creates, saves, deletes, sends, moves…);\n\
         - `capability_calls`: a Capability that, by its name or role, can only work by calling \
         another Capability.\n\
         For each, the units it is about, why (one sentence), and the fix you propose (one \
         sentence an agent can apply: rename, merge, move to `actions`, split). Report nothing \
         else, and nothing you are not sure of. Then give the effect of every DataCapability \
         listed under `actions`.\n\n",
    );
    let _ = write!(
        text,
        "Write `why` and `fix` in {language}. Answer with one JSON object and nothing else: \
         {{\"findings\": [{{\"check\": \"duplicate\", \"units\": [\"system.verbThing\"], \"why\": \
         \"…\", \"fix\": \"…\"}}], \"effects\": {{\"system.verbThing\": \"insert\"}}}}.\n\n\
         The cut (micro-frontend — system — what it does — needs — actions):\n\n"
    );
    for c in &ctx.components {
        let _ = writeln!(
            text,
            "- `{}` — {} — {} — {} — {}",
            c.name,
            c.system,
            c.description.trim(),
            c.needs.join(", "),
            c.actions.join(", ")
        );
    }
    text
}

/// The guard's answer, as the AI gives it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Answer {
    findings: Vec<Said>,
    effects: BTreeMap<String, String>,
}

/// One finding as the AI words it: a check or an effect it names wrong
/// drops that one entry, not the whole answer.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Said {
    check: String,
    units: Vec<String>,
    why: String,
    fix: String,
}

/// A `snake_case` name read as one of `T`'s variants.
fn variant<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(name.trim().to_string())).ok()
}

/// The audit of a cut: the checks read off it first, then the AI's, those
/// it names a check it has not dropped; numbered in that order. Effects
/// are kept for the cut's own writes only.
///
/// # Errors
///
/// The AI's answer carries no JSON object, or one that does not read.
pub fn audit_of(answer: &str, components: &[ComponentRef], from: &str) -> Result<Audit, String> {
    let start = answer.find('{').ok_or("the guard answered nothing")?;
    let end = answer.rfind('}').ok_or("the guard answered nothing")?;
    let object = answer
        .get(start..=end)
        .ok_or("the guard answered nothing")?;
    let read: Answer = serde_json::from_str(object)
        .map_err(|e| format!("the guard's answer is not readable: {e}"))?;
    let writes: BTreeSet<&str> = components
        .iter()
        .flat_map(|c| c.actions.iter())
        .map(String::as_str)
        .collect();
    let ai = read.findings.into_iter().filter_map(|f| {
        let check: Check = variant(&f.check)?;
        matches!(
            check,
            Check::Duplicate | Check::WriteAsRead | Check::CapabilityCalls
        )
        .then(|| Finding {
            id: 0,
            check: Some(check),
            units: f.units,
            why: f.why,
            fix: f.fix,
        })
    });
    let mut findings: Vec<Finding> = local_findings(components).into_iter().chain(ai).collect();
    for (n, f) in findings.iter_mut().enumerate() {
        f.id = u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1);
    }
    Ok(Audit {
        findings,
        effects: read
            .effects
            .into_iter()
            .filter(|(unit, _)| writes.contains(unit.as_str()))
            .filter_map(|(unit, effect)| Some((unit, variant(&effect)?)))
            .collect(),
        from: from.to_string(),
    })
}

/// The question that applies one finding's fix to the mock-up, in its
/// workspace: the units renamed, merged or moved in `mock/pages.json` and
/// in the fragments, the micro-frontends' names kept.
#[must_use]
pub fn fix_prompt(rules: &str, finding: &Finding) -> String {
    let mut text = rules.to_string();
    let _ = write!(
        text,
        "\nThe architecture's guard found this on the mock-up's cut into micro-frontends, \
         Capabilities and DataCapabilities:\n\n- about: {}\n- why: {}\n- fix: {}\n\n\
         Apply the fix in the mock-up: in `mock/pages.json` (each micro-frontend's `system`, \
         `needs`, `actions`) and in the fragments under `mock/components/` that name the units \
         (`data-reads`, `data-writes`). Keep every micro-frontend's name, every page and what \
         the human sees. Change nothing else. Say in one line what you changed.\n",
        finding.units.join(", "),
        finding.why.trim(),
        finding.fix.trim()
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mf(name: &str, system: &str, needs: &[&str], actions: &[&str]) -> ComponentRef {
        ComponentRef {
            name: name.to_string(),
            system: system.to_string(),
            description: String::new(),
            needs: needs.iter().map(ToString::to_string).collect(),
            actions: actions.iter().map(ToString::to_string).collect(),
            ..ComponentRef::default()
        }
    }

    fn of(found: &[Finding], check: Check) -> Vec<String> {
        found
            .iter()
            .filter(|f| f.check == Some(check))
            .flat_map(|f| f.units.clone())
            .collect()
    }

    #[test]
    fn a_fragment_that_names_no_part_for_a_need_or_an_action_blocks() {
        let mut named = mf(
            "Pjs",
            "campagne",
            &["campagne.listerPjs"],
            &["campagne.ajouterPj"],
        );
        named.fragment = "components/Pjs.html".to_string();
        named.markup = r#"<ul data-reads="campagne.listerPjs"></ul><button data-writes="campagne.ajouterPj">+</button>"#.to_string();
        let mut half = named.clone();
        half.name = "Half".to_string();
        half.markup = r#"<ul data-reads="campagne.listerPjs"></ul><button>+</button>"#.to_string();
        let unread = mf("Unread", "campagne", &["campagne.listerPjs"], &[]);
        let found = unnamed_parts(&[named, half, unread]);
        assert_eq!(
            found.len(),
            1,
            "a fragment not read is not judged: {found:?}"
        );
        assert_eq!(found[0].check, Some(Check::PartUnnamed));
        assert_eq!(found[0].units, ["Half", "campagne.ajouterPj"]);
        assert!(
            found[0].why.contains("campagne.ajouterPj") && found[0].fix.contains("data-writes")
        );
        assert!(found[0].blocks());
        let mut audit = Audit {
            findings: vec![Finding {
                id: 1,
                ..found[0].clone()
            }],
            ..Audit::default()
        };
        assert!(
            audit
                .resolve(1, false)
                .expect_err("not dismissed")
                .contains("named, not dismissed")
        );
        assert!(audit.resolve(1, true).is_ok(), "named by its fix");
    }

    #[test]
    fn a_unit_not_named_system_dot_verb_thing_is_found_once() {
        let cut = [
            mf(
                "List",
                "campagne",
                &["campagne.listerCampagnes", "Campagne.Lire"],
                &["campagne"],
            ),
            mf(
                "Card",
                "campagne",
                &["Campagne.Lire"],
                &["campagne.creer-campagne"],
            ),
        ];
        assert_eq!(
            of(&local_findings(&cut), Check::Naming),
            vec!["Campagne.Lire", "campagne", "campagne.creer-campagne"]
        );
        let naming = local_findings(&cut);
        let first = naming
            .iter()
            .find(|f| f.units == ["Campagne.Lire"])
            .expect("found");
        assert_eq!(
            first.why,
            "`Campagne.Lire` is not named `<system>.<verbThing>`"
        );
        assert_eq!(
            first.fix,
            "rename `Campagne.Lire` as `<system>.<verbThing>` wherever it is listed"
        );
        assert!(well_named("ia.choisirFournisseur"));
        assert!(well_named("sauve-garde.exporter"));
        assert!(!well_named("sauve garde.exporter"));
        assert!(well_named("sauve_garde.exporter2"));
        assert!(!well_named(".lire"));
        assert!(!well_named("ia."));
        assert!(!well_named("ia.Lire"));
    }

    #[test]
    fn a_system_too_big_or_alone_on_its_micro_frontend_is_found() {
        let mut cut: Vec<ComponentRef> = (0..=SYSTEM_MAX)
            .map(|i| mf(&format!("M{i}"), "big", &[], &[]))
            .collect();
        cut.push(mf("Lone", "thin", &[], &[]));
        cut.push(mf("A", "fit", &[], &[]));
        cut.push(mf("B", "fit", &[], &[]));
        let found = local_findings(&cut);
        assert_eq!(of(&found, Check::SystemSize), vec!["big", "thin"]);
        let said: Vec<(&str, &str)> = found
            .iter()
            .filter(|f| f.check == Some(Check::SystemSize))
            .map(|f| (f.why.as_str(), f.fix.as_str()))
            .collect();
        assert_eq!(
            said,
            vec![
                (
                    "`big` holds 9 micro-frontends: more than one bounded context",
                    "split `big` into the bounded contexts it holds"
                ),
                (
                    "`thin` holds one micro-frontend only",
                    "keep `thin` if it is a context of its own, else fold it into the system it serves"
                ),
            ]
        );
        let exactly: Vec<ComponentRef> = (0..SYSTEM_MAX)
            .map(|i| mf(&format!("M{i}"), "big", &[], &[]))
            .collect();
        assert!(of(&local_findings(&exactly), Check::SystemSize).is_empty());
        assert!(
            of(
                &local_findings(&[mf("Only", "one", &[], &[])]),
                Check::SystemSize
            )
            .is_empty(),
            "a product of one system is not thin"
        );
    }

    #[test]
    fn a_micro_frontend_reaching_into_another_system_is_found() {
        let cut = [
            mf(
                "Table",
                "table",
                &["table.lireCarte", "campagne.listerPersonnages"],
                &["journal.noter"],
            ),
            mf("Home", "table", &["table.lireCarte"], &["table.deplacer"]),
        ];
        let found = local_findings(&cut);
        assert_eq!(of(&found, Check::MixedSystems), vec!["Table"]);
        let why = &found
            .iter()
            .find(|f| f.check == Some(Check::MixedSystems))
            .expect("found")
            .why;
        assert_eq!(why, "`Table` (table) reaches into campagne, journal");
        let fix = &found
            .iter()
            .find(|f| f.check == Some(Check::MixedSystems))
            .expect("found")
            .fix;
        assert_eq!(
            fix,
            "move `Table` to the system it serves, or give `table` the units it needs"
        );
    }

    #[test]
    fn the_audit_numbers_the_local_findings_then_the_ai_s_and_keeps_the_cut_s_effects() {
        let cut = [mf(
            "List",
            "campagne",
            &["campagne.lister", "campagne.creer"],
            &[
                "campagne.ajouter",
                "campagne.modifier",
                "campagne.supprimer",
            ],
        )];
        let answer = r#"Here: {"findings": [
            {"check": "write_as_read", "units": ["campagne.creer"], "why": "it creates", "fix": "move it to actions"},
            {"check": "naming", "units": ["x"], "why": "the AI does not judge names"},
            {"check": "nonsense", "units": ["y"]}
        ], "effects": {"campagne.ajouter": "insert", "campagne.modifier": "update", "campagne.supprimer": "explode", "elsewhere.x": "delete"}}"#;
        let audit = audit_of(answer, &cut, "cut one").expect("read");
        assert_eq!(audit.from, "cut one");
        assert_eq!(audit.findings.len(), 1);
        assert_eq!(audit.findings[0].id, 1);
        assert_eq!(audit.findings[0].check, Some(Check::WriteAsRead));
        assert_eq!(audit.effects.len(), 2);
        assert_eq!(audit.effects["campagne.ajouter"], Effect::Insert);
        assert_eq!(audit.effects["campagne.modifier"], Effect::Update);
        assert_eq!(audit.blocking().len(), 1);

        let lone = [mf("Lone", "a", &["Bad"], &[]), mf("Two", "b", &[], &[])];
        let audit = audit_of(
            r#"{"findings": [{"check": "duplicate", "units": ["a.x", "a.y"]}]}"#,
            &lone,
            "",
        )
        .expect("read");
        let ids: Vec<u32> = audit.findings.iter().map(|f| f.id).collect();
        assert_eq!(
            ids,
            (1..=u32::try_from(ids.len()).expect("few")).collect::<Vec<_>>()
        );
        assert_eq!(
            audit.findings.last().and_then(|f| f.check),
            Some(Check::Duplicate)
        );
        assert!(audit.blocking().is_empty());
        assert!(audit_of("no json", &lone, "").is_err());
        assert!(audit_of("{not json}", &lone, "").is_err());
    }

    #[test]
    fn only_a_write_passed_off_as_a_read_blocks_and_it_is_fixed_not_dismissed() {
        for check in [
            Check::Duplicate,
            Check::Naming,
            Check::SystemSize,
            Check::MixedSystems,
            Check::CapabilityCalls,
        ] {
            assert!(!check.blocks(), "{check:?}");
        }
        assert!(Check::WriteAsRead.blocks());
        let mut audit = Audit {
            findings: vec![
                Finding {
                    id: 1,
                    check: Some(Check::Naming),
                    ..Finding::default()
                },
                Finding {
                    id: 2,
                    check: Some(Check::WriteAsRead),
                    ..Finding::default()
                },
            ],
            ..Audit::default()
        };
        assert!(audit.resolve(2, false).is_err());
        assert!(audit.resolve(9, true).is_err());
        assert_eq!(audit.resolve(1, false).expect("dismissed").id, 1);
        assert_eq!(audit.resolve(2, true).expect("applied").id, 2);
        assert!(audit.findings.is_empty());
        assert!(!Finding::default().blocks());
    }

    #[test]
    fn the_guard_s_questions_carry_the_rules_the_cut_and_the_fix() {
        let ctx = Context {
            components: vec![mf(
                "List",
                "campagne",
                &["campagne.lister"],
                &["campagne.ajouter"],
            )],
            ..Context::default()
        };
        let asked = audit_prompt(&ctx, "French");
        assert!(asked.contains("strictly read-only") && asked.contains("`write_as_read`"));
        assert!(asked.contains("Write `why` and `fix` in French"));
        assert!(asked.contains("- `List` — campagne —  — campagne.lister — campagne.ajouter"));
        let fix = fix_prompt(
            "RULES",
            &Finding {
                units: vec!["a.x".to_string(), "a.y".to_string()],
                why: " same ".to_string(),
                fix: " merge ".to_string(),
                ..Finding::default()
            },
        );
        assert!(fix.starts_with("RULES\n"));
        assert!(fix.contains("- about: a.x, a.y\n- why: same\n- fix: merge"));
        assert!(fix.contains("Keep every micro-frontend's name"));
    }
}
