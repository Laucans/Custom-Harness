//! The API of a dependency the checkout builds but does not track: what a
//! unit calls of `harness-dataguard`, carried into the prompt of the tasks
//! that use it.
//!
//! The product takes the engine by `git` at a release tag
//! (`tag = "dataguard-v5"`), so its source is never among the tracked files
//! the signature index reads, and five data-layer sessions out of eight
//! went digging through `~/.cargo/git` for it — about 39 turns. The index of
//! the dependency is built from what cargo resolved: the lock file's
//! `source = "git+…?tag=…#<sha>"` names the exact source, which a tag never
//! moves, so the same lock line always means the same API and nothing has to
//! be kept fresh — a tag bump is read by the next run.
//!
//! Which tasks carry it ([`carries`]): every task on the write side — an
//! aggregate, a `DataCapability`, the data layer — and every task whose body
//! names the engine (`DataGuard`, `harness_dataguard`, …), an infrastructure
//! task under `crates/dataguard/` included.
//!
//! What it carries ([`carried_dependencies`]): the five modules a unit
//! implements against ([`DATAGUARD_UNIT_MODULES`], `lib` first for its
//! `Error` and its re-exports) whatever the product's use says, then the
//! others in the order the product uses them — counted from its
//! `harness_dataguard::<module>` paths, test code included, which ranks
//! `memory`, `api` and `engine` high — up to [`DEPENDENCY_BUDGET`], apart
//! from the product files' own budget so neither starves the other. The
//! modules left out are named, never their path on disk: the point is that
//! the session need not go there.
//!
//! Pure: the launcher reads the lock file and the sources, this decides.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::rust_shape::{Visibility, shape};

/// The engine's crate, as the lock file names it.
pub const DATAGUARD: &str = "harness-dataguard";

/// The engine's crate, as the product's code names it.
pub const DATAGUARD_IDENT: &str = "harness_dataguard";

/// The words a task body names the engine with, compared whole and without
/// case: "safeguard" and "guard" are not among them.
pub const DATAGUARD_WORDS: [&str; 4] = [
    "dataguard",
    "data-guard",
    "harness_dataguard",
    "harness-dataguard",
];

/// The engine's modules a unit implements against, carried ahead of the
/// product's usage order: ranked by use alone, `aggregate` tied with
/// `memory` behind `api` on `ForgeDesMondes` and was the one left out.
pub const DATAGUARD_UNIT_MODULES: [&str; 5] =
    ["lib", "plugins", "manifest", "aggregate", "command"];

/// How much a dependency's API may weigh in a prompt, in characters (about
/// 2.5k tokens): on top of the product files' own [`BUDGET`](super::signatures::BUDGET).
///
/// Measured on the engine as this workspace holds it: its public surface
/// shapes to about 27k characters over 18 files (17 modules); the five a unit
/// uses ([`DATAGUARD_UNIT_MODULES`]) take about 9k with their block
/// wrappers, carried first; what is left goes to the product's next most
/// used module that fits. Over a hundred turns it costs about $0.08 of cache
/// reads a session, against the eight turns of reading it spares.
pub const DEPENDENCY_BUDGET: usize = 10_000;

/// One module of a dependency, shaped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Module {
    /// Its name: `lib`, `plugins`, `postgres` (its sub-files folded in).
    pub name: String,
    /// Its file under the crate, `src/plugins.rs`.
    pub path: String,
    /// Its public shape.
    pub shape: String,
}

/// A dependency's API, its modules in the order they are carried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dependency {
    /// The crate's name.
    pub name: String,
    /// The version the checkout builds: `dataguard-v5 @ 56cb6be`, or a path.
    pub source: String,
    /// Its modules, `lib` first, then by the product's use.
    pub modules: Vec<Module>,
}

/// A `git` dependency as the lock file resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locked {
    /// The repository.
    pub url: String,
    /// The tag the manifest names, when it names one.
    pub tag: Option<String>,
    /// The commit cargo resolved.
    pub rev: String,
}

impl Locked {
    /// How the prompt names this version: the tag and the short commit.
    #[must_use]
    pub fn label(&self) -> String {
        let short: String = self.rev.chars().take(7).collect();
        self.tag
            .as_ref()
            .map_or_else(|| short.clone(), |tag| format!("{tag} @ {short}"))
    }
}

/// The `git` source the lock file resolves for the package `name`: `None`
/// for a package it takes from a path or a registry, or that it lacks.
#[must_use]
pub fn locked(lock: &str, name: &str) -> Option<Locked> {
    for package in lock.split("[[package]]").skip(1) {
        let field = |key: &str| {
            package.lines().find_map(|line| {
                let (left, right) = line.split_once('=')?;
                (left.trim() == key).then(|| right.trim().trim_matches('"').to_string())
            })
        };
        if field("name").as_deref() != Some(name) {
            continue;
        }
        let source = field("source")?;
        let rest = source.strip_prefix("git+")?;
        let (location, rev) = rest.split_once('#')?;
        let (url, query) = location.split_once('?').unwrap_or((location, ""));
        let tag = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("tag="))
            .map(ToString::to_string);
        return Some(Locked {
            url: url.to_string(),
            tag,
            rev: rev.to_string(),
        });
    }
    None
}

/// The manifest path and the id of the package `name` in the output of
/// `cargo metadata --format-version 1`, when it lists it.
#[must_use]
pub fn package_in_metadata(metadata: &str, name: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(metadata).ok()?;
    value["packages"].as_array()?.iter().find_map(|package| {
        (package["name"].as_str() == Some(name)).then(|| {
            (
                package["manifest_path"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                package["id"].as_str().unwrap_or_default().to_string(),
            )
        })
    })
}

/// The module a source file under `src/` belongs to: `lib.rs` is `lib`,
/// `postgres/values.rs` is `postgres`. `None` for what is not Rust.
#[must_use]
pub fn module_of(relative: &str) -> Option<String> {
    let path = relative.strip_suffix(".rs")?;
    let first = path.split('/').next().filter(|first| !first.is_empty())?;
    Some(first.to_string())
}

/// The modules ranked by how often `sources` use them, `lib` first.
///
/// Counted as `ident::<module>` (or in a `ident::{…}` list), then ranked by
/// count, ties by name — the same order every run, so the prompt's cache
/// holds. A name with no module (a re-export, `Error`) is not counted.
#[must_use]
pub fn usage_order(sources: &[String], ident: &str, modules: &[String]) -> Vec<String> {
    let mut count: BTreeMap<&str, usize> = modules.iter().map(|m| (m.as_str(), 0)).collect();
    let prefix = format!("{ident}::");
    let mut tally = |name: &str| {
        if let Some(n) = count.get_mut(name) {
            *n += 1;
        }
    };
    for source in sources {
        let mut rest = source.as_str();
        while let Some(at) = rest.find(&prefix) {
            rest = &rest[at + prefix.len()..];
            if let Some(group) = rest.strip_prefix('{') {
                let inside = group.split('}').next().unwrap_or_default();
                for item in inside.split(',') {
                    tally(item.trim().split("::").next().unwrap_or_default().trim());
                }
            } else {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                tally(&name);
            }
        }
    }
    let mut ranked: Vec<(&str, usize)> = count.into_iter().collect();
    ranked.sort_by(|(a, x), (b, y)| {
        (*b == "lib")
            .cmp(&(*a == "lib"))
            .then(y.cmp(x))
            .then(a.cmp(b))
    });
    ranked
        .into_iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

/// The API of a dependency from its source files, shaped module by module.
///
/// `files` are `(path under src/, text)`; the `first` modules come in their
/// order, then the others by how the checkout's own `product` sources use
/// them.
#[must_use]
pub fn dependency(
    name: &str,
    source: &str,
    files: &[(String, String)],
    product: &[String],
    ident: &str,
    first: &[&str],
) -> Dependency {
    let mut by_module: BTreeMap<String, Module> = BTreeMap::new();
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    // A module's own file before its sub-files.
    sorted.sort_by_key(|(path, _)| (path.matches('/').count(), path.clone()));
    for (path, text) in sorted {
        let Some(module) = module_of(path) else {
            continue;
        };
        let shaped = shape(text, Visibility::External);
        if shaped.trim().is_empty() {
            continue;
        }
        let entry = by_module.entry(module.clone()).or_insert_with(|| Module {
            name: module,
            path: format!("src/{path}"),
            shape: String::new(),
        });
        if !entry.shape.is_empty() {
            entry.shape.push('\n');
        }
        entry.shape.push_str(&shaped);
    }
    let names: Vec<String> = by_module.keys().cloned().collect();
    let ranked = usage_order(product, ident, &names);
    let modules = first
        .iter()
        .map(|name| (*name).to_string())
        .chain(ranked)
        .filter_map(|name| by_module.remove(&name))
        .collect();
    Dependency {
        name: name.to_string(),
        source: source.to_string(),
        modules,
    }
}

/// Whether `body` names one of `words`, whole and without case.
#[must_use]
pub fn names_dependency(body: &str, words: &[&str]) -> bool {
    body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .any(|token| words.iter().any(|word| token.eq_ignore_ascii_case(word)))
}

/// Whether a task carries the engine's API: every write-side task, and
/// every task whose body names the engine.
#[must_use]
pub fn carries(body: &str, write_side: bool) -> bool {
    write_side || names_dependency(body, &DATAGUARD_WORDS)
}

/// The dependencies' API as the prompt carries it, within `budget`: one
/// block per module in their order, a module that does not fit left out and
/// named at the end.
#[must_use]
pub fn carried_dependencies(dependencies: &[Dependency], budget: usize) -> String {
    let mut out = String::new();
    for dependency in dependencies {
        let mut left_out: Vec<&str> = Vec::new();
        for module in &dependency.modules {
            let block = format!(
                "<signatures crate=\"{}\" source=\"{}\" path=\"{}\">\n{}\n</signatures>",
                dependency.name, dependency.source, module.path, module.shape
            );
            if out.len() + block.len() + 2 > budget {
                left_out.push(&module.name);
                continue;
            }
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&block);
        }
        if !left_out.is_empty() {
            let _ = write!(
                out,
                "\n[... {} module(s) of {} not carried, the block hit its budget: {} ...]",
                left_out.len(),
                dependency.name,
                left_out.join(", ")
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"
[[package]]
name = "harness-dataguard"
version = "0.1.0"
source = "git+https://github.com/laucans/custom-harness?tag=dataguard-v5#56cb6be9f54a5b29dae0cdf8e544600eaa63c556"
dependencies = [
 "serde",
]

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    #[test]
    fn the_lock_file_names_the_tag_and_the_commit_cargo_resolved() {
        let found = locked(LOCK, DATAGUARD).expect("locked");
        assert_eq!(found.url, "https://github.com/laucans/custom-harness");
        assert_eq!(found.tag.as_deref(), Some("dataguard-v5"));
        assert_eq!(found.rev, "56cb6be9f54a5b29dae0cdf8e544600eaa63c556");
        assert_eq!(found.label(), "dataguard-v5 @ 56cb6be");
        assert_eq!(
            locked(LOCK, "serde"),
            None,
            "a registry package is not a git one"
        );
        assert_eq!(locked(LOCK, "absent"), None);
        let path_only = "[[package]]\nname = \"harness-dataguard\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            locked(path_only, DATAGUARD),
            None,
            "a path dependency has no source"
        );
    }

    #[test]
    fn a_metadata_answer_gives_the_manifest_and_the_id() {
        let metadata = r#"{"packages":[{"name":"serde","manifest_path":"/s/Cargo.toml","id":"s"},
            {"name":"harness-dataguard","manifest_path":"/x/crates/dataguard/Cargo.toml","id":"git+https://g?tag=dataguard-v5#56cb6be"}]}"#;
        assert_eq!(
            package_in_metadata(metadata, DATAGUARD),
            Some((
                "/x/crates/dataguard/Cargo.toml".to_string(),
                "git+https://g?tag=dataguard-v5#56cb6be".to_string()
            ))
        );
        assert_eq!(package_in_metadata("not json", DATAGUARD), None);
        assert_eq!(package_in_metadata(r#"{"packages":[]}"#, DATAGUARD), None);
    }

    #[test]
    fn a_file_belongs_to_its_top_module() {
        assert_eq!(module_of("lib.rs").as_deref(), Some("lib"));
        assert_eq!(module_of("postgres/values.rs").as_deref(), Some("postgres"));
        assert_eq!(module_of("README.md"), None);
    }

    fn strings(of: &[&str]) -> Vec<String> {
        of.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_modules_rank_by_use_lib_first_ties_by_name() {
        let product = strings(&[
            "use harness_dataguard::plugins::Prepare;\nuse harness_dataguard::{manifest, plugins::Judge};",
            "harness_dataguard::plugins::Context; harness_dataguard::Error; harness_dataguard::tokio_postgres::Client;",
        ]);
        let modules = strings(&["aggregate", "lib", "manifest", "plugins", "queue"]);
        assert_eq!(
            usage_order(&product, DATAGUARD_IDENT, &modules),
            strings(&["lib", "plugins", "manifest", "aggregate", "queue"])
        );
        // The same answer however the modules came.
        let shuffled = strings(&["queue", "plugins", "lib", "aggregate", "manifest"]);
        assert_eq!(
            usage_order(&product, DATAGUARD_IDENT, &shuffled),
            usage_order(&product, DATAGUARD_IDENT, &modules)
        );
    }

    #[test]
    fn the_engine_is_named_whole_and_without_case() {
        for body in [
            "the DataGuard refuses it",
            "use harness_dataguard::plugins",
            "harness-dataguard = { git = … }",
            "under crates/dataguard/server",
            "a Data-Guard rule",
        ] {
            assert!(names_dependency(body, &DATAGUARD_WORDS), "{body}");
        }
        for body in ["a guard", "the safeguard holds", "dataguards"] {
            assert!(!names_dependency(body, &DATAGUARD_WORDS), "{body}");
        }
        assert!(
            carries("a screen", true),
            "the write side always carries it"
        );
        assert!(!carries("a screen", false));
    }

    fn engine() -> Dependency {
        Dependency {
            name: DATAGUARD.to_string(),
            source: "dataguard-v5 @ 56cb6be".to_string(),
            modules: ["lib", "plugins", "queue"]
                .iter()
                .map(|name| Module {
                    name: (*name).to_string(),
                    path: format!("src/{name}.rs"),
                    shape: format!("pub fn {name}_api()\n{}", "x".repeat(300)),
                })
                .collect(),
        }
    }

    #[test]
    fn each_module_is_a_block_naming_the_crate_and_its_version() {
        let said = carried_dependencies(&[engine()], DEPENDENCY_BUDGET);
        assert!(
            said.contains(
                "<signatures crate=\"harness-dataguard\" source=\"dataguard-v5 @ 56cb6be\" path=\"src/plugins.rs\">"
            ),
            "{said}"
        );
        assert!(said.find("lib_api").unwrap_or(usize::MAX) < said.find("plugins_api").unwrap_or(0));
        assert!(!said.contains("not carried"), "{said}");
        assert_eq!(carried_dependencies(&[], DEPENDENCY_BUDGET), "");
    }

    #[test]
    fn a_module_over_the_budget_is_named_and_its_path_never_given() {
        let said = carried_dependencies(&[engine()], 1_000);
        assert!(said.len() <= 1_000 + 150, "{}", said.len());
        assert!(said.contains("lib_api"), "{said}");
        assert!(
            said.contains(
                "1 module(s) of harness-dataguard not carried, the block hit its budget: queue"
            ),
            "{said}"
        );
        assert!(!said.contains(".cargo"), "{said}");
    }

    /// The engine's sources as this workspace holds them.
    fn workspace_engine() -> Vec<(String, String)> {
        macro_rules! files {
            ($($path:literal),*) => {
                vec![$(($path.to_string(), include_str!(concat!("../../../../dataguard/src/", $path)).to_string())),*]
            };
        }
        files!(
            "lib.rs",
            "aggregate.rs",
            "api.rs",
            "command.rs",
            "engine.rs",
            "guard.rs",
            "manifest.rs",
            "memory.rs",
            "messages.rs",
            "plugins.rs",
            "postgres.rs",
            "postgres/values.rs",
            "queue.rs",
            "reads.rs",
            "reconcile.rs",
            "registry.rs",
            "resolver.rs",
            "store.rs"
        )
    }

    #[test]
    fn the_engine_s_api_carries_what_a_unit_implements_within_its_budget() {
        // The product's use, shaped as measured on ForgeDesMondes: `api` and
        // `memory` (test code) rank high, `aggregate` ties with `memory` —
        // ranked by use alone, `aggregate` was the module left out.
        let product: Vec<String> = [
            ("plugins", 1_500),
            ("api", 1_200),
            ("command", 1_000),
            ("manifest", 900),
            ("aggregate", 770),
            ("memory", 770),
            ("engine", 300),
        ]
        .iter()
        .map(|(module, n)| format!("harness_dataguard::{module}::X;\n").repeat(*n))
        .collect();
        let engine = dependency(
            DATAGUARD,
            "dataguard-v5 @ 56cb6be",
            &workspace_engine(),
            &product,
            DATAGUARD_IDENT,
            &DATAGUARD_UNIT_MODULES,
        );
        let order: Vec<&str> = engine.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            &order[..6],
            &["lib", "plugins", "manifest", "aggregate", "command", "api"],
            "the unit's modules first, then by use: {order:?}"
        );
        assert_eq!(
            engine.modules.len(),
            17,
            "postgres/values.rs folds into postgres"
        );
        let said = carried_dependencies(&[engine], DEPENDENCY_BUDGET);
        assert!(said.len() <= DEPENDENCY_BUDGET + 200, "{}", said.len());
        for module in ["lib", "plugins", "manifest", "aggregate", "command"] {
            assert!(
                said.contains(&format!("path=\"src/{module}.rs\"")),
                "{module} carried whole: {said}"
            );
        }
        assert!(
            said.contains(
                "async fn prepare(&self, context: &Context<'_>, payload: &BTreeMap<String, Value>, target_id: Option<&str>) -> Result<Prepared, Refusal>;"
            ),
            "{said}"
        );
        assert!(
            said.contains("not carried"),
            "the server's internals come last: {said}"
        );
    }

    proptest::proptest! {
        #[test]
        fn any_lock_text_reads_without_panicking(lock in "\\PC{0,400}") {
            let _ = locked(&lock, DATAGUARD);
            let _ = package_in_metadata(&lock, DATAGUARD);
        }

        #[test]
        fn a_generated_git_package_round_trips(
            host in "[a-z]{1,10}",
            repo in "[a-z][a-z-]{0,12}",
            tag in proptest::option::of("[a-z0-9.-]{1,16}"),
            rev in "[0-9a-f]{40}",
            before in "[a-z]{1,8}",
        ) {
            let url = format!("https://{host}.com/{repo}");
            let query = tag.as_ref().map_or_else(String::new, |tag| format!("?tag={tag}"));
            let lock = format!(
                "[[package]]\nname = \"{before}\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"{DATAGUARD}\"\nversion = \"0.1.0\"\nsource = \"git+{url}{query}#{rev}\"\n"
            );
            let found = locked(&lock, DATAGUARD);
            proptest::prop_assert_eq!(found, Some(Locked { url, tag, rev }));
        }

        #[test]
        fn any_source_ranks_without_panicking_lib_first_and_the_same_every_time(
            sources in proptest::collection::vec("(\\PC|harness_dataguard::(plugins|lib|queue|\\{plugins, queue\\}|é)){0,40}", 0..5),
            mut modules in proptest::collection::vec("[a-z]{1,8}", 0..6),
        ) {
            modules.push("lib".to_string());
            modules.sort();
            modules.dedup();
            let ranked = usage_order(&sources, DATAGUARD_IDENT, &modules);
            proptest::prop_assert_eq!(ranked.first().map(String::as_str), Some("lib"));
            proptest::prop_assert_eq!(ranked.len(), modules.len());
            let mut reversed = modules.clone();
            reversed.reverse();
            proptest::prop_assert_eq!(&ranked, &usage_order(&sources, DATAGUARD_IDENT, &reversed));
        }

        #[test]
        fn a_file_s_module_is_its_first_path_segment(path in "[a-z_]{1,8}(/[a-z_]{1,8}){0,3}", ext in "(rs|md|toml)") {
            let file = format!("{path}.{ext}");
            let first = path.split('/').next().map(ToString::to_string);
            proptest::prop_assert_eq!(module_of(&file), first.filter(|_| ext == "rs"));
        }
    }
}
