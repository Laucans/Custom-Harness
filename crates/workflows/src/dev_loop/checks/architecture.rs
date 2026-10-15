//! Does the checkout still follow the agent-native architecture once `code`
//! is done? Two gates on the round, both deterministic and both local.
//!
//! The reading and the rules are [`harness_architecture::tree`]; these gates
//! only read the checkout through the `Disk` port, hand the rules the task's
//! own `## Architecture` declaration, and stop the round with the list.
//!
//! [`ArchitectureHolds`] checks every rule a diff is reviewed against, and that
//! the unit the task declared is there, on the side it said.
//! [`ConceptsDocumented`] checks that every Concept a Capability implements is
//! documented: meaning is what is shared between systems, and a Concept nobody
//! wrote down is a copy nobody can check.
//!
//! The CI gates of the repository run the same rules; these are the loop's
//! own reading, so a round that drifted stops here with the list rather than
//! at the next pull request's checks. A checkout without `contracts/` is not
//! under the architecture: both gates say so and let the round through.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_architecture::closure;
use harness_architecture::tree::{Tree, undocumented_concepts, violations};
use harness_architecture::vocabulary::Declaration;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repo;

use crate::common::sections;
use crate::dev_loop::data::state::Loop;

/// The task's own declaration, read from its body.
fn declaration_of(ctx: &Context<Loop>) -> Option<Declaration> {
    sections::parse(&ctx.state.task.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
}

/// The files a round may touch outside its unit's closure: what every
/// change to the workspace updates, and what a unit registers itself in.
pub const SHARED: [&str; 7] = [
    "Cargo.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "queries/registry.json",
    "data/model.json",
    "mock/pages.json",
];

/// True when `path` is one a round may touch whatever its unit: a lockfile,
/// a registry, the server's manifest (a `DataCapability` registers itself on
/// its dependency line), a bundle the build script rewrites.
#[must_use]
pub fn shared(path: &str) -> bool {
    SHARED.contains(&path)
        || (path.starts_with("crates/") && path.ends_with("/server/Cargo.toml"))
        || path.starts_with("mock/components/")
}

/// The files the round changed that its unit's closure does not hold.
///
/// The drift the architecture forbids — a task that built its unit and
/// "fixed" a neighbour on the way. Nothing when the task declares no unit,
/// the unit reaches the whole workspace, or the base is unknown.
#[must_use]
pub fn outside_closure(
    tree: &Tree,
    declaration: Option<&Declaration>,
    changed: &[String],
) -> Vec<String> {
    let Some(declaration) = declaration else {
        return Vec::new();
    };
    let found = closure::closure(tree, declaration, None, tree.root_manifest.as_deref());
    if !found.narrowed {
        return Vec::new();
    }
    changed
        .iter()
        .filter(|path| !path.trim().is_empty())
        .filter(|path| !shared(path) && !closure::allows(&found.patterns, path))
        .cloned()
        .collect()
}

/// The paths `git status --porcelain` lists, without their status columns.
fn dirty_paths(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| line.get(3..).map(str::trim).map(ToString::to_string))
        .filter(|path| !path.is_empty())
        .collect()
}

/// The round kept the architecture: see the module.
pub struct ArchitectureHolds {
    /// What reads the checkout.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
    /// The lane's checkout, for what the round changed.
    pub repo: Rc<dyn Repo>,
    /// The branch the round's pull request merged into — where its changes
    /// are, once `code` has merged them.
    pub integration_branch: String,
}

#[async_trait(?Send)]
impl Verification<Loop> for ArchitectureHolds {
    fn purpose(&self) -> String {
        "static read of the checkout: every Capability stays isolated, every manifest names its system, every Micro-UI needs Capabilities of its own system, and the unit the task declared is there on the side it said".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        if !tree.under_architecture {
            ctx.traces
                .say("architecture: no contracts/ in this checkout — not under the architecture, gate skipped");
            return Ok(Verdict::Continue);
        }
        let declaration = declaration_of(ctx);
        let mut found = violations(&tree, declaration.as_ref());
        // The round's own changes, against the task's closure: what the
        // session touched that its unit does not hold.
        if !ctx.state.round_base.is_empty() {
            let mut changed = self
                .repo
                .changed_between(
                    &ctx.state.round_base,
                    &format!("origin/{}", self.integration_branch),
                )
                .await?;
            changed.extend(dirty_paths(&self.repo.dirty_files().await?));
            for path in outside_closure(&tree, declaration.as_ref(), &changed) {
                found.push(format!(
                    "{path}: changed by this round, outside the closure of the {} it builds — a \
                     unit touches its own folder, what it registers in, and nothing of its \
                     neighbours",
                    declaration.as_ref().map_or("unit", |d| d.unit.key())
                ));
            }
        }
        if found.is_empty() {
            ctx.traces.say(&format!(
                "architecture holds: {} Capability crate(s), {} Micro-UI(s), {} DataCapabilit(ies)",
                tree.capabilities.len(),
                tree.micro_uis.len(),
                tree.data_capabilities.len()
            ));
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "the architecture no longer holds after #{} — fix these before anything else builds \
             on them (the repository's gates check the same rules):\n  - {}",
            ctx.state.task.number,
            found.join("\n  - ")
        )))
    }
}

/// Every Concept a Capability implements is documented: see the module.
pub struct ConceptsDocumented {
    /// What reads the checkout.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl Verification<Loop> for ConceptsDocumented {
    fn purpose(&self) -> String {
        "static read of the checkout: every Concept a Capability implements has its concepts/<Name>/v<n>/concept.json".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        if !tree.under_architecture {
            return Ok(Verdict::Continue);
        }
        let found = undocumented_concepts(&tree);
        if found.is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "a Capability implements a Concept nobody documented — write \
             concepts/<Concept>/v<n>/concept.json (contract A) and its fixtures first:\n  - {}",
            found.join("\n  - ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::dev_loop::data::state::Loop;
    use harness_core::domain::Named;
    use harness_core::execution::Settings;
    use harness_core::ports::shell::git::Repos as _;
    use harness_core::traces::Logbook;

    const MICRO_UI: &str = "## Architecture\n\nunit: micro-ui\nsystem: credit\nproduces: RiskBadge\nneeds: credit.calculateRisk\n";

    fn disk() -> FakeDisk {
        let mut disk = FakeDisk::default();
        for (path, content) in [
            ("/ws/contracts/README.md", "contracts"),
            (
                "/ws/crates/credit/capabilities/calculate-risk/capability.json",
                r#"{"capability":"credit.calculateRisk","system":"credit","reads":["q.graphql"]}"#,
            ),
            (
                "/ws/crates/credit/capabilities/calculate-risk/Cargo.toml",
                "[package]\n",
            ),
            (
                "/ws/crates/credit/capabilities/calculate-risk/q.graphql",
                "# fields: a.b\nquery Q { a }",
            ),
            (
                "/ws/apps/credit/risk-badge/micro-ui.json",
                r#"{"microUi":"RiskBadge","system":"credit","needs":["credit.calculateRisk"]}"#,
            ),
            (
                "/ws/mock/pages.json",
                r#"{"version":1,"pages":[{"path":"pages/credit.html","title":"Credit","objective":"o","components":["RiskBadge"]}],"components":[{"name":"RiskBadge","system":"credit","description":"d","fragment":"components/RiskBadge.html","needs":["credit.calculateRisk"],"actions":[]}]}"#,
            ),
            (
                "/ws/mock/components/RiskBadge.html",
                r#"<div data-reads="credit.calculateRisk"></div>"#,
            ),
            (
                "/ws/mock/pages/credit.html",
                r#"<div data-component="RiskBadge"></div>"#,
            ),
        ] {
            disk.existing
                .insert(PathBuf::from(path), content.to_string());
        }
        let query = "# fields: a.b\nquery Q { a }";
        disk.existing.insert(
            PathBuf::from("/ws/queries/registry.json"),
            format!(
                r#"{{"queries":[{{"query":"Q","capability":"credit.calculateRisk","file":"crates/credit/capabilities/calculate-risk/q.graphql","sha256":"{}","fields":["a.b"]}}]}}"#,
                harness_architecture::tree::sha256(query)
            ),
        );
        disk
    }

    fn tree() -> Tree {
        Tree::read(&disk(), std::path::Path::new("/ws"))
    }

    fn declaration() -> Declaration {
        Declaration::parse(
            sections::parse(MICRO_UI)
                .get(sections::ARCHITECTURE)
                .expect("declared"),
        )
        .expect("a unit")
    }

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn a_change_inside_the_closure_or_shared_is_fine_and_a_neighbour_s_is_not() {
        let drift = outside_closure(
            &tree(),
            Some(&declaration()),
            &files(&[
                "apps/credit/risk-badge/index.ts",
                "mock/pages/credit-review.html",
                "mock/pages.json",
                "Cargo.lock",
                "mock/components/RiskBadge.js",
                "crates/dataguard/server/Cargo.toml",
                "crates/credit/capabilities/calculate-risk/src/lib.rs",
                "docs/notes.md",
                "",
            ]),
        );
        assert_eq!(
            drift,
            ["crates/credit/capabilities/calculate-risk/src/lib.rs"],
            "a neighbour's code; docs/ is in every closure"
        );
    }

    #[test]
    fn nothing_drifts_without_a_declaration_or_for_a_unit_that_reaches_everything() {
        let changed = files(&["anything/at/all.rs"]);
        assert!(
            outside_closure(&tree(), None, &changed).is_empty(),
            "expected nothing"
        );
        let tooling = Declaration {
            unit: harness_architecture::vocabulary::Unit::Tooling,
            ..declaration()
        };
        assert!(
            outside_closure(&tree(), Some(&tooling), &changed).is_empty(),
            "expected nothing"
        );
    }

    #[test]
    fn porcelain_lines_lose_their_status_columns() {
        assert_eq!(
            dirty_paths(&files(&[
                " M apps/x.ts",
                "?? new/file.rs",
                "A  added.rs",
                ""
            ])),
            ["apps/x.ts", "new/file.rs", "added.rs"]
        );
    }

    fn ctx(base: &str) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            Loop {
                task: Named {
                    number: "34".to_string(),
                    title: "RiskBadge".to_string(),
                    body: MICRO_UI.to_string(),
                },
                round_base: base.to_string(),
                ..Loop::default()
            },
            Logbook::null(),
        )
    }

    fn holds(repos: &FakeRepos) -> ArchitectureHolds {
        ArchitectureHolds {
            disk: Rc::new(disk()),
            root: PathBuf::from("/ws"),
            repo: repos.at(std::path::Path::new("/ws")),
            integration_branch: "milestone/3-risk".to_string(),
        }
    }

    #[tokio::test]
    async fn a_round_that_touched_a_neighbour_stops_naming_the_file() {
        let repos = FakeRepos::changing(&["crates/credit/capabilities/calculate-risk/src/lib.rs"]);
        let err = holds(&repos)
            .verify(&ctx("abc123"))
            .await
            .expect_err("drifted");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(
            err.reason().contains("calculate-risk/src/lib.rs"),
            "{}",
            err.reason()
        );
        assert!(
            err.reason().contains("outside the closure of the micro-ui"),
            "{}",
            err.reason()
        );
        assert!(
            repos.asked("diff abc123..origin/milestone/3-risk"),
            "{:?}",
            repos.calls()
        );
    }

    #[tokio::test]
    async fn a_round_that_stayed_in_its_closure_goes_on() {
        let repos = FakeRepos::changing(&["apps/credit/risk-badge/index.ts", "mock/pages.json"]);
        let verdict = holds(&repos).verify(&ctx("abc123")).await.expect("holds");
        assert!(matches!(verdict, Verdict::Continue));
    }

    #[tokio::test]
    async fn an_unknown_base_judges_the_tree_but_not_the_diff() {
        let repos = FakeRepos::changing(&["crates/credit/capabilities/calculate-risk/src/lib.rs"]);
        holds(&repos)
            .verify(&ctx(""))
            .await
            .expect("no diff to judge");
        assert!(!repos.asked("diff"), "{:?}", repos.calls());
    }
}
