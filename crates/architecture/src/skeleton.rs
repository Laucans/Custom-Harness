//! The skeleton of the unit a task is about to build.
//!
//! Its folder is written from the harness's templates before the session
//! opens, so the session starts from the product's canonical shape instead
//! of copying a neighbour's — and its drift with it.
//!
//! Two kinds only, the data layer's: a `DataCapability` and a Capability.
//! A skeleton fills what needs no judgement — the crate's `Cargo.toml`, the
//! manifest's constants, the `lib.rs` frame, the test frames on the
//! harness's test kit (`crates/platform/test-kit`), the persisted query's
//! frame named after the folder — and marks every judged part with
//! [`MARKER`]: the manifest's `payload`, `mode`, `invariants`, the SQL, the
//! business rules, one test per acceptance criterion. A Rust hole compiles,
//! so the skeleton passes `cargo check` and fails the unit's own tests until
//! it is filled. Only the `DataCapability` manifest's `mode` (and an
//! undeclared `effect`) is refused by its schema; every other hole — a
//! Capability manifest's, a doc comment, the Cargo `description` — passes
//! the schemas and compiles, so the product's `holes` rule
//! (`scripts/unit-conventions-gate.mjs`) refuses any [`MARKER`] left in an
//! added or changed unit, repair rounds and `pr_fix` included.
//!
//! Pure: [`target`] reads the declaration, [`render`] the tree a caller
//! read; the dev loop's `Scaffold` writes the files, runs [`Skeleton::commands`]
//! and takes everything back when one fails.

use std::fmt;

use crate::tree::{Tree, snake};
use crate::vocabulary::{Declaration, Effect, Unit};

/// What marks a part of a skeleton the session must write: none may be
/// left once the task is done.
pub const MARKER: &str = "SCAFFOLD:";

/// The templates' version, said in the journal beside a skeleton.
pub const VERSION: u32 = 1;

/// The files the skeleton's commands may rewrite beside its own: what a
/// failed skeleton puts back as it found them.
pub const REWRITTEN: [&str; 4] = [
    "Cargo.toml",
    "Cargo.lock",
    "crates/dataguard/registry/Cargo.toml",
    "queries/registry.json",
];

const DC_CARGO: &str = include_str!("../assets/skeleton/data-capability/Cargo.toml");
const DC_MANIFEST: &str = include_str!("../assets/skeleton/data-capability/data-capability.json");
const DC_LIB: &str = include_str!("../assets/skeleton/data-capability/src/lib.rs");
const DC_PREPARE: &str = include_str!("../assets/skeleton/data-capability/tests/prepare.rs");
const DC_ENGINE: &str = include_str!("../assets/skeleton/data-capability/tests/engine.rs");
const DC_POSTGRES: &str = include_str!("../assets/skeleton/data-capability/tests/postgres.rs");
const CAP_CARGO: &str = include_str!("../assets/skeleton/capability/Cargo.toml");
const CAP_MANIFEST: &str = include_str!("../assets/skeleton/capability/capability.json");
const CAP_LIB: &str = include_str!("../assets/skeleton/capability/src/lib.rs");
const CAP_QUERY: &str = include_str!("../assets/skeleton/capability/query.graphql");

/// Where a unit goes and what it is called, from its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// A `DataCapability` or a Capability.
    pub unit: Unit,
    /// `<system>.<name>`.
    pub id: String,
    /// The id's namespace, its folder under `crates/`.
    pub system: String,
    /// The id's name, `camelCase`.
    pub name: String,
    /// The name in `kebab-case`: the unit's folder and its package.
    pub folder: String,
    /// `crates/<system>/<kind>/<folder>`.
    pub dir: String,
    /// The package's library, the folder with `_` for `-`.
    pub lib: String,
    /// A Capability's persisted query, `crates/<system>/queries/<folder>.graphql`.
    pub query: Option<String>,
}

/// Why no skeleton is written: the session builds the unit as before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// The unit is not one the templates cover.
    Unsupported(Unit),
    /// `produces` names no single `<system>.<name>` id.
    NoId(Vec<String>),
    /// The unit runs in no server: compiled for its host, its shape differs.
    Hostless,
    /// The id's namespace is not the declared system.
    OtherSystem(String),
    /// A `DataCapability` whose declaration names no aggregate.
    NoAggregate,
    /// The aggregate it names has no crate in the checkout.
    UnknownAggregate(String),
    /// A unit of the checkout already has the package's name.
    Taken(String),
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(unit) => write!(f, "no template for a {} unit", unit.key()),
            Self::NoId(ids) => write!(
                f,
                "`produces` names no single `<system>.<name>` id ({})",
                if ids.is_empty() {
                    "nothing".to_string()
                } else {
                    ids.join(", ")
                }
            ),
            Self::Hostless => f.write_str("a hostless unit has a shape of its own"),
            Self::OtherSystem(id) => write!(f, "`{id}` is not of the declared system"),
            Self::NoAggregate => f.write_str("the declaration names no aggregate"),
            Self::UnknownAggregate(name) => {
                write!(f, "the aggregate `{name}` has no crate in the checkout")
            }
            Self::Taken(package) => write!(f, "a unit is already named `{package}`"),
        }
    }
}

/// A unit's files and what makes them part of the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skeleton {
    /// Each file, its path from the root and its content.
    pub files: Vec<(String, String)>,
    /// What runs once the files are written, in order: the registry, the
    /// queries' registry, the compile check of the crate and its tests.
    pub commands: Vec<Vec<String>>,
}

impl Skeleton {
    /// The holes the session fills: every [`MARKER`] of every file.
    #[must_use]
    pub fn holes(&self) -> usize {
        self.files
            .iter()
            .map(|(_, text)| text.matches(MARKER).count())
            .sum()
    }
}

/// `camelCase` as `kebab-case`, the way the registry's build script reads a
/// folder back (`tree::snake`, `-` for `_`).
#[must_use]
pub fn kebab(name: &str) -> String {
    snake(name).replace('_', "-")
}

/// The id `declaration` produces and where it goes — or why there is none.
///
/// # Errors
///
/// A [`Skip`] when the unit is not a `DataCapability` or a Capability, runs
/// in no server, or names no single id of its own system.
pub fn target(declaration: &Declaration) -> Result<Target, Skip> {
    let kind = match declaration.unit {
        Unit::DataCapability => "data-capabilities",
        Unit::Capability => "capabilities",
        other => return Err(Skip::Unsupported(other)),
    };
    if declaration
        .host
        .as_deref()
        .is_some_and(|host| host.eq_ignore_ascii_case("hostless"))
    {
        return Err(Skip::Hostless);
    }
    let ids: Vec<&String> = declaration
        .produces
        .iter()
        .filter(|id| id.contains('.'))
        .collect();
    let [id] = ids.as_slice() else {
        return Err(Skip::NoId(declaration.produces.clone()));
    };
    let Some((system, name)) = id.split_once('.') else {
        return Err(Skip::NoId(declaration.produces.clone()));
    };
    let well_formed = |part: &str| {
        part.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && part.chars().all(|c| c.is_ascii_alphanumeric())
    };
    if !well_formed(name) || !system.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return Err(Skip::NoId(declaration.produces.clone()));
    }
    if system.is_empty() || (!declaration.system.is_empty() && declaration.system != system) {
        return Err(Skip::OtherSystem((*id).clone()));
    }
    let folder = kebab(name);
    let dir = format!("crates/{system}/{kind}/{folder}");
    let query = (declaration.unit == Unit::Capability).then(|| query_path(system, &folder));
    Ok(Target {
        unit: declaration.unit,
        id: (*id).clone(),
        system: system.to_string(),
        name: name.to_string(),
        lib: folder.replace('-', "_"),
        folder,
        dir,
        query,
    })
}

/// A Capability's persisted query: named after its folder, under its
/// system's `queries/`.
fn query_path(system: &str, folder: &str) -> String {
    format!("crates/{system}/queries/{folder}.graphql")
}

/// An aggregate's crate, as a `DataCapability` depends on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateCrate {
    /// The aggregate's name.
    pub name: String,
    /// Its folder, `crates/<system>/aggregates/<Name>`.
    pub dir: String,
    /// Its package.
    pub package: String,
}

impl AggregateCrate {
    /// Its library: the package with `_` for `-`.
    #[must_use]
    pub fn lib(&self) -> String {
        self.package.replace('-', "_")
    }
}

/// The package a `Cargo.toml` names in its `[package]` section.
fn package_name(toml: &str) -> Option<String> {
    let mut section = "";
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed;
            continue;
        }
        if section != "[package]" {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=')
            && key.trim() == "name"
        {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// The crate of the aggregate `name`, in whichever system the tree holds it.
#[must_use]
pub fn aggregate_crate(tree: &Tree, name: &str) -> Option<AggregateCrate> {
    let manifest = tree.aggregates.iter().find(|m| {
        m.json
            .as_ref()
            .and_then(|json| json.get("aggregate"))
            .and_then(serde_json::Value::as_str)
            == Some(name)
    })?;
    let dir = manifest.path.strip_suffix("/aggregate.json")?.to_string();
    let package = tree
        .unit_crates
        .iter()
        .find(|unit| unit.dir == dir)
        .and_then(|unit| package_name(&unit.cargo_toml))?;
    Some(AggregateCrate {
        name: name.to_string(),
        dir,
        package,
    })
}

/// The path from the folder `from` to the folder `to`, both from the root.
fn relative(from: &str, to: &str) -> String {
    let from: Vec<&str> = from.split('/').filter(|p| !p.is_empty()).collect();
    let to: Vec<&str> = to.split('/').filter(|p| !p.is_empty()).collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

/// `text` with each `{{key}}` replaced by its value.
fn fill(text: &str, values: &[(&str, &str)]) -> String {
    values.iter().fold(text.to_string(), |text, (key, value)| {
        text.replace(&format!("{{{{{key}}}}}"), value)
    })
}

/// A JSON string literal of `text`.
fn quoted(text: &str) -> String {
    serde_json::Value::String(text.to_string()).to_string()
}

/// The manifest's `effect` and `touches`, as the declaration states them:
/// a hole for what it leaves open.
fn effect_and_touches(declaration: &Declaration, aggregate: &str) -> (String, String) {
    let hole = || {
        format!(
            "[{}]",
            quoted(&format!(
                "{MARKER} the existing fields of {aggregate} it changes, as {aggregate}.<field>"
            ))
        )
    };
    let Some(effect) = declaration.effect else {
        return (
            quoted(&format!("{MARKER} insert | update | delete | upsert")),
            hole(),
        );
    };
    let touches = if declaration.touches.is_empty() {
        if effect == Effect::Insert {
            "[]".to_string()
        } else {
            hole()
        }
    } else {
        let fields: Vec<String> = declaration
            .touches
            .iter()
            .map(|touch| {
                if touch.contains('.') {
                    quoted(touch)
                } else {
                    quoted(&format!("{aggregate}.{touch}"))
                }
            })
            .collect();
        format!("[{}]", fields.join(", "))
    };
    (quoted(effect.key()), touches)
}

/// The skeleton of `target`: its files and the commands that register and
/// check it.
///
/// `aggregate` is a `DataCapability`'s target crate, `projects` whether that
/// aggregate writes PostgreSQL tables (its tests then get a PostgreSQL
/// frame).
///
/// # Errors
///
/// A [`Skip`] when a unit of `tree` already has the package's name, or a
/// `DataCapability` names no aggregate the tree holds.
pub fn render(
    target: &Target,
    declaration: &Declaration,
    tree: &Tree,
    aggregate: Option<&AggregateCrate>,
    projects: bool,
) -> Result<Skeleton, Skip> {
    if tree
        .unit_crates
        .iter()
        .filter_map(|unit| package_name(&unit.cargo_toml))
        .any(|package| package == target.folder)
    {
        return Err(Skip::Taken(target.folder.clone()));
    }
    let common = [
        ("id", target.id.as_str()),
        ("system", target.system.as_str()),
        ("folder", target.folder.as_str()),
        ("pkg", target.folder.as_str()),
        ("lib", target.lib.as_str()),
    ];
    let register = vec!["bash".to_string(), "scripts/register.sh".to_string()];
    let check = vec![
        "cargo".to_string(),
        "check".to_string(),
        "--quiet".to_string(),
        "-p".to_string(),
        target.folder.clone(),
        "--all-targets".to_string(),
    ];
    match target.unit {
        Unit::DataCapability => {
            let Some(name) = declaration.aggregate.as_deref() else {
                return Err(Skip::NoAggregate);
            };
            let Some(aggregate) = aggregate.filter(|a| a.name == name) else {
                return Err(Skip::UnknownAggregate(name.to_string()));
            };
            Ok(Skeleton {
                files: data_capability_files(target, declaration, aggregate, projects, &common),
                commands: vec![register, check],
            })
        }
        Unit::Capability => Ok(Skeleton {
            files: capability_files(target, declaration, &common),
            commands: vec![
                register,
                vec!["bash".to_string(), "scripts/queries.sh".to_string()],
                check,
            ],
        }),
        other => Err(Skip::Unsupported(other)),
    }
}

/// A `DataCapability`'s files, its aggregate's crate known.
fn data_capability_files(
    target: &Target,
    declaration: &Declaration,
    aggregate: &AggregateCrate,
    projects: bool,
    common: &[(&str, &str)],
) -> Vec<(String, String)> {
    let dir = &target.dir;
    let (effect, touches) = effect_and_touches(declaration, &aggregate.name);
    let agg_dir = relative(dir, &aggregate.dir);
    let agg_lib = aggregate.lib();
    let mut values = common.to_vec();
    values.extend([
        ("Agg", aggregate.name.as_str()),
        ("agg_pkg", aggregate.package.as_str()),
        ("agg_lib", agg_lib.as_str()),
        ("agg_dir", agg_dir.as_str()),
        ("agg_dir_root", aggregate.dir.as_str()),
        ("effect", effect.as_str()),
        ("touches", touches.as_str()),
    ]);
    let mut files = vec![
        (format!("{dir}/Cargo.toml"), fill(DC_CARGO, &values)),
        (
            format!("{dir}/data-capability.json"),
            fill(DC_MANIFEST, &values),
        ),
        (format!("{dir}/src/lib.rs"), fill(DC_LIB, &values)),
        (format!("{dir}/tests/prepare.rs"), fill(DC_PREPARE, &values)),
        (format!("{dir}/tests/engine.rs"), fill(DC_ENGINE, &values)),
    ];
    if projects {
        files.push((
            format!("{dir}/tests/postgres.rs"),
            fill(DC_POSTGRES, &values),
        ));
    }
    files
}

/// A Capability's files: its crate and its persisted query.
fn capability_files(
    target: &Target,
    declaration: &Declaration,
    common: &[(&str, &str)],
) -> Vec<(String, String)> {
    let dir = &target.dir;
    let pascal = {
        let mut chars = target.name.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_ascii_uppercase().to_string() + chars.as_str()
        })
    };
    let concept = declaration.concept.as_deref().filter(|c| !c.is_empty());
    let implements = concept.map_or_else(String::new, |c| {
        format!("\n  \"implements\": {},", quoted(c))
    });
    let dependency = concept.map_or_else(String::new, |_| {
        "\nplatform-conformance = { path = \"../../../platform/conformance\" }".to_string()
    });
    let conformance = concept.map_or_else(String::new, |c| {
        format!(
            "\n\n    #[test]\n    fn conformance() {{\n        platform_conformance::check({}, |input| {{\n            let _ = input;\n            panic!(\"{MARKER} the Concept's computation, local to this Capability\")\n        }});\n    }}",
            quoted(c)
        )
    });
    let mut values = common.to_vec();
    values.extend([
        ("Pascal", pascal.as_str()),
        ("implements", implements.as_str()),
        ("conformance_dependency", dependency.as_str()),
        ("conformance_test", conformance.as_str()),
    ]);
    let query = query_path(&target.system, &target.folder);
    vec![
        (format!("{dir}/Cargo.toml"), fill(CAP_CARGO, &values)),
        (
            format!("{dir}/capability.json"),
            fill(CAP_MANIFEST, &values),
        ),
        (format!("{dir}/src/lib.rs"), fill(CAP_LIB, &values)),
        (query, fill(CAP_QUERY, &values)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Manifest, UnitCrate};
    use proptest::prelude::*;

    fn declaration(text: &str) -> Declaration {
        Declaration::parse(text).expect("a declaration")
    }

    const MOVE: &str = "unit: data-capability\nsystem: table\neffect: update\ntouches: q, BattlemapInstance.r\naggregate: BattlemapInstance\nproduces: table.moveToken\n";
    const LIST: &str = "unit: capability\nsystem: campaign\nconcept: AliasMatch@1\nproduces: campaign.listEntitiesToCreate\n";

    fn tree() -> Tree {
        Tree {
            aggregates: vec![Manifest {
                path: "crates/table/aggregates/BattlemapInstance/aggregate.json".to_string(),
                json: Some(serde_json::json!({ "aggregate": "BattlemapInstance" })),
            }],
            unit_crates: vec![
                UnitCrate {
                    dir: "crates/table/aggregates/BattlemapInstance".to_string(),
                    cargo_toml: "[package]\nname = \"aggregate-battlemap-instance\"\n\n[dependencies]\nname = \"x\"\n".to_string(),
                    hostless: false,
                },
                UnitCrate {
                    dir: "crates/table/data-capabilities/end-combat".to_string(),
                    cargo_toml: "[package]\nname = \"end-combat\"\n".to_string(),
                    hostless: false,
                },
            ],
            ..Tree::default()
        }
    }

    fn file<'a>(skeleton: &'a Skeleton, suffix: &str) -> &'a str {
        skeleton
            .files
            .iter()
            .find(|(path, _)| path.ends_with(suffix))
            .map_or_else(|| panic!("no {suffix}"), |(_, text)| text.as_str())
    }

    #[test]
    fn a_data_capability_goes_under_its_id_s_system_named_after_its_name() {
        let target = target(&declaration(MOVE)).expect("a target");
        assert_eq!(target.dir, "crates/table/data-capabilities/move-token");
        assert_eq!(target.lib, "move_token");
        assert_eq!(target.query, None);
        let cap = super::target(&declaration(LIST)).expect("a target");
        assert_eq!(
            cap.dir,
            "crates/campaign/capabilities/list-entities-to-create"
        );
        assert_eq!(
            cap.query.as_deref(),
            Some("crates/campaign/queries/list-entities-to-create.graphql")
        );
    }

    #[test]
    fn what_the_templates_do_not_cover_is_skipped_and_said() {
        for (text, skip) in [
            (
                "unit: micro-ui\nsystem: table\nproduces: Battlemap\n",
                Skip::Unsupported(Unit::MicroUi),
            ),
            (
                "unit: capability\nsystem: ai\nhost: hostless\nproduces: ai.listRequests\n",
                Skip::Hostless,
            ),
            (
                "unit: capability\nsystem: ai\nproduces: ai.a, ai.b\n",
                Skip::NoId(vec!["ai.a".to_string(), "ai.b".to_string()]),
            ),
            ("unit: capability\nsystem: ai\n", Skip::NoId(Vec::new())),
            (
                "unit: capability\nsystem: ai\nproduces: ai.Bad\n",
                Skip::NoId(vec!["ai.Bad".to_string()]),
            ),
            (
                "unit: capability\nsystem: ai\nproduces: AI.list\n",
                Skip::NoId(vec!["AI.list".to_string()]),
            ),
            (
                "unit: capability\nsystem: ai\nproduces: table.list\n",
                Skip::OtherSystem("table.list".to_string()),
            ),
        ] {
            assert_eq!(
                super::target(&declaration(text)),
                Err(skip.clone()),
                "{text}"
            );
            assert_ne!(skip.to_string(), "");
        }
        assert!(Skip::NoId(Vec::new()).to_string().contains("nothing"));
        assert!(Skip::Taken("x".into()).to_string().contains("`x`"));
    }

    #[test]
    fn a_data_capability_s_skeleton_fills_what_the_declaration_states() {
        let declaration = declaration(MOVE);
        let target = target(&declaration).expect("a target");
        let tree = tree();
        let aggregate = aggregate_crate(&tree, "BattlemapInstance").expect("its crate");
        assert_eq!(aggregate.package, "aggregate-battlemap-instance");
        assert_eq!(aggregate.lib(), "aggregate_battlemap_instance");
        let skeleton =
            render(&target, &declaration, &tree, Some(&aggregate), false).expect("rendered");
        let manifest: serde_json::Value =
            serde_json::from_str(file(&skeleton, "data-capability.json")).expect("JSON");
        assert_eq!(manifest["dataCapability"], "table.moveToken");
        assert_eq!(manifest["owner"], "dataguard");
        assert_eq!(manifest["effect"], "update");
        assert_eq!(
            manifest["touches"],
            serde_json::json!(["BattlemapInstance.q", "BattlemapInstance.r"])
        );
        assert_eq!(manifest["target"]["aggregate"], "BattlemapInstance");
        for judged in ["mode", "invariants", "payload"] {
            assert!(manifest[judged].to_string().contains(MARKER), "{judged}");
        }
        let cargo = file(&skeleton, "Cargo.toml");
        assert!(cargo.contains("name = \"move-token\""));
        assert!(cargo.contains(
            "aggregate-battlemap-instance = { path = \"../../aggregates/BattlemapInstance\" }"
        ));
        assert!(cargo.contains("platform-test-kit = { path = \"../../../platform/test-kit\" }"));
        assert!(
            file(&skeleton, "tests/engine.rs")
                .contains("&file(\"crates/table/aggregates/BattlemapInstance/aggregate.json\")")
        );
        // A unit includes nothing outside its folder (scripts/isolation.sh).
        for (path, text) in &skeleton.files {
            for (at, _) in text.match_indices("include_str!(\"") {
                let included = &text[at + 14..];
                assert!(
                    included.starts_with("../data-capability.json\")"),
                    "{path} includes {}",
                    included.split('"').next().unwrap_or_default()
                );
            }
        }
        assert!(
            file(&skeleton, "tests/engine.rs")
                .contains("[aggregate_battlemap_instance::plugin(), move_token::plugin()]")
        );
        assert!(
            file(&skeleton, "src/lib.rs")
                .contains("pub const DATA_CAPABILITY: &str = \"table.moveToken\";")
        );
        assert!(
            !skeleton
                .files
                .iter()
                .any(|(p, _)| p.ends_with("postgres.rs"))
        );
        assert!(skeleton.files.iter().all(|(_, text)| !text.contains("{{")));
        assert!(skeleton.holes() > 5, "{}", skeleton.holes());
        assert_eq!(skeleton.commands[0], ["bash", "scripts/register.sh"]);
        assert_eq!(
            skeleton.commands[1],
            [
                "cargo",
                "check",
                "--quiet",
                "-p",
                "move-token",
                "--all-targets"
            ]
        );
        let projected =
            render(&target, &declaration, &tree, Some(&aggregate), true).expect("rendered");
        assert!(
            file(&projected, "tests/postgres.rs").contains("Scratch::new(\"move_token_tables\")")
        );
    }

    #[test]
    fn an_aggregate_of_another_system_is_reached_by_its_path() {
        let declaration = declaration(
            "unit: data-capability\nsystem: session\neffect: insert\ntouches: -\naggregate: BattlemapInstance\nproduces: session.openMap\n",
        );
        let target = target(&declaration).expect("a target");
        let tree = tree();
        let aggregate = aggregate_crate(&tree, "BattlemapInstance");
        let skeleton =
            render(&target, &declaration, &tree, aggregate.as_ref(), false).expect("rendered");
        assert!(
            file(&skeleton, "Cargo.toml")
                .contains("{ path = \"../../../table/aggregates/BattlemapInstance\" }")
        );
        let manifest: serde_json::Value =
            serde_json::from_str(file(&skeleton, "data-capability.json")).expect("JSON");
        assert_eq!(manifest["effect"], "insert");
        assert_eq!(manifest["touches"], serde_json::json!([]));
    }

    #[test]
    fn what_the_declaration_leaves_open_is_a_hole() {
        let tree = tree();
        let aggregate = aggregate_crate(&tree, "BattlemapInstance");
        for text in [
            "unit: data-capability\nsystem: table\naggregate: BattlemapInstance\nproduces: table.a\n",
            "unit: data-capability\nsystem: table\neffect: update\ntouches: -\naggregate: BattlemapInstance\nproduces: table.a\n",
        ] {
            let declaration = declaration(text);
            let target = target(&declaration).expect("a target");
            let skeleton =
                render(&target, &declaration, &tree, aggregate.as_ref(), false).expect("rendered");
            let manifest: serde_json::Value =
                serde_json::from_str(file(&skeleton, "data-capability.json")).expect("JSON");
            assert!(manifest["touches"].to_string().contains(MARKER), "{text}");
        }
        let open = declaration(
            "unit: data-capability\nsystem: table\naggregate: BattlemapInstance\nproduces: table.a\n",
        );
        let skeleton = render(
            &target(&open).expect("a target"),
            &open,
            &tree,
            aggregate.as_ref(),
            false,
        )
        .expect("rendered");
        assert!(file(&skeleton, "data-capability.json").contains("\"effect\": \"SCAFFOLD:"));
    }

    #[test]
    fn a_data_capability_needs_its_aggregate_s_crate_and_a_free_name() {
        let tree = tree();
        let no_aggregate = declaration("unit: data-capability\nsystem: table\nproduces: table.a\n");
        let target_a = target(&no_aggregate).expect("a target");
        assert_eq!(
            render(&target_a, &no_aggregate, &tree, None, false),
            Err(Skip::NoAggregate)
        );
        let unknown = declaration(
            "unit: data-capability\nsystem: table\naggregate: Ghost\nproduces: table.a\n",
        );
        let aggregate = aggregate_crate(&tree, "BattlemapInstance");
        assert_eq!(
            render(
                &target(&unknown).expect("a target"),
                &unknown,
                &tree,
                aggregate.as_ref(),
                false
            ),
            Err(Skip::UnknownAggregate("Ghost".to_string()))
        );
        assert_eq!(aggregate_crate(&tree, "Ghost"), None);
        let taken = declaration(
            "unit: data-capability\nsystem: table\naggregate: BattlemapInstance\nproduces: table.endCombat\n",
        );
        assert_eq!(
            render(
                &target(&taken).expect("a target"),
                &taken,
                &tree,
                aggregate.as_ref(),
                false
            ),
            Err(Skip::Taken("end-combat".to_string()))
        );
    }

    #[test]
    fn a_capability_s_skeleton_names_its_query_after_its_folder() {
        let wanted = declaration(LIST);
        let target = target(&wanted).expect("a target");
        let skeleton = render(&target, &wanted, &Tree::default(), None, false).expect("rendered");
        let manifest: serde_json::Value =
            serde_json::from_str(file(&skeleton, "capability.json")).expect("JSON");
        assert_eq!(manifest["capability"], "campaign.listEntitiesToCreate");
        assert_eq!(manifest["system"], "campaign");
        assert_eq!(manifest["implements"], "AliasMatch@1");
        assert_eq!(
            manifest["reads"],
            serde_json::json!(["queries/list-entities-to-create.graphql"])
        );
        let query = file(&skeleton, "queries/list-entities-to-create.graphql");
        assert!(query.starts_with("# fields: SCAFFOLD:"));
        assert!(query.contains("query ListEntitiesToCreate($scaffold: ID!)"));
        assert!(
            file(&skeleton, "Cargo.toml")
                .contains("platform-conformance = { path = \"../../../platform/conformance\" }")
        );
        assert!(
            file(&skeleton, "src/lib.rs").contains("platform_conformance::check(\"AliasMatch@1\"")
        );
        assert!(
            file(&skeleton, "src/lib.rs")
                .contains("read::query(\"campaign\", \"list-entities-to-create\")")
        );
        assert!(
            !file(&skeleton, "src/lib.rs").contains("include_str!"),
            "isolation.sh: nothing outside its folder"
        );
        assert_eq!(skeleton.commands[1], ["bash", "scripts/queries.sh"]);
        assert_eq!(skeleton.commands.len(), 3);
        let plain =
            declaration("unit: capability\nsystem: campaign\nproduces: campaign.getThing\n");
        let skeleton = render(
            &super::target(&plain).expect("a target"),
            &plain,
            &Tree::default(),
            None,
            false,
        )
        .expect("rendered");
        assert!(!file(&skeleton, "capability.json").contains("implements"));
        assert!(!file(&skeleton, "Cargo.toml").contains("conformance"));
        assert!(!file(&skeleton, "src/lib.rs").contains("conformance"));
        serde_json::from_str::<serde_json::Value>(file(&skeleton, "capability.json"))
            .expect("JSON");
    }

    #[test]
    fn the_relative_path_climbs_to_the_common_folder() {
        assert_eq!(
            relative("crates/a/k/x", "crates/a/aggregates/X"),
            "../../aggregates/X"
        );
        assert_eq!(
            relative("crates/a/k/x/tests", "crates/b/aggregates/X"),
            "../../../../b/aggregates/X"
        );
        assert_eq!(relative("a/b", "a/b"), ".");
        assert_eq!(package_name("[dependencies]\nname = \"x\"\n"), None);
    }

    proptest! {
        #[test]
        fn a_folder_reads_back_as_the_library_the_registry_spells(name in "[a-z][a-z0-9]{0,6}([A-Z][a-z0-9]{1,6}){0,3}") {
            let folder = kebab(&name);
            prop_assert_eq!(folder.replace('-', "_"), snake(&name));
            prop_assert!(!folder.contains("--") && !folder.starts_with('-'));
            prop_assert!(folder.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        }
    }
}
