//! Narrowing the working tree to the task's closure, and widening it back.
//!
//! The bound the architecture promises is made physical here: before the
//! stages that plan and build a unit, the lane's checkout is reduced to the
//! closure of that unit (`harness_architecture::closure`) with
//! `git sparse-checkout`; after them, the whole tree comes back so the
//! post-gates read everything. What the session cannot read, it cannot
//! pay for — and cannot drift into.
//!
//! Both are unpaid actions in the stage's own body: [`Narrow`] first,
//! [`Widen`] last, around the paid `Ask`. A stage interrupted between the
//! two leaves a narrowed clone, which the next mount widens before it reads
//! anything (`provisioning::reuse`).

use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use async_trait::async_trait;
use harness_architecture::closure::{self, CargoGraph};
use harness_architecture::tree::Tree;
use harness_architecture::vocabulary::Declaration;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::process::Process;

use crate::common::sections;
use crate::dev_loop::data::state::Loop;

/// How long `cargo metadata` may take to answer; a workspace that resolves
/// does so in seconds, and one that does not is read as no graph.
const METADATA_TIMEOUT: Duration = Duration::from_secs(90);

/// Narrows the checkout to the closure of the task's unit.
pub struct Narrow {
    /// The lane's checkout.
    pub repo: Rc<dyn Repo>,
    /// What runs `cargo metadata` in it.
    pub process: Rc<dyn Process>,
    /// What reads the checkout's manifests.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
}

impl Narrow {
    /// The workspace's crate graph, or `None` when `cargo metadata` does not
    /// answer — a product with no Cargo workspace, or one that does not
    /// resolve: the closure then narrows by folder.
    async fn graph(&self) -> Option<CargoGraph> {
        let args = ["metadata", "--format-version", "1", "--no-deps"]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let ran = self
            .process
            .run("cargo", &args, &self.root, METADATA_TIMEOUT)
            .await
            .ok()?;
        if !ran.ok() {
            return None;
        }
        CargoGraph::parse(&ran.stdout)
    }
}

/// The paths a session of this task stopped on as missing —
/// `AGENT_LOOP_STOP: missing <path>`, kept in the body by the decision that
/// answered it. The prompt promises the harness widens the closure to them;
/// the declaration alone never would, and the task would stop again.
fn asked_for(body: &str) -> Vec<String> {
    let mut paths: Vec<String> = body
        .lines()
        .filter_map(|line| {
            line.split_once("AGENT_LOOP_STOP: missing ")
                .map(|(_, after)| after)
        })
        .flat_map(|after| after.split(|c: char| c.is_whitespace() || c == ','))
        .map(|word| {
            word.trim_matches(|c: char| matches!(c, '`' | '(' | ')' | ':' | ';' | '.' | '\'' | '"'))
        })
        .filter(|word| {
            word.contains('/')
                && !word.starts_with('/')
                && !word.contains("..")
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
        })
        .map(|word| word.trim_end_matches('/').to_string())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// The task's declaration, as its body carries it.
fn declaration_of(body: &str) -> Option<Declaration> {
    sections::parse(body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
}

#[async_trait(?Send)]
impl Action<Loop> for Narrow {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        ctx.state.tree_note = String::new();
        let Some(declaration) = declaration_of(&ctx.state.task.body) else {
            ctx.traces
                .say("tree: the task declares no unit — the whole checkout stays");
            return Ok(Verdict::Continue);
        };
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        let graph = self.graph().await;
        let found = closure::closure(
            &tree,
            &declaration,
            graph.as_ref(),
            tree.root_manifest.as_deref(),
        );
        // An id a delivered sibling produces is held, whether or not the
        // tree has a place to index it.
        let missing: Vec<&String> = found
            .missing
            .iter()
            .filter(|id| {
                !ctx.state
                    .delivered
                    .iter()
                    .any(|held| held.eq_ignore_ascii_case(id))
            })
            .collect();
        if !missing.is_empty() {
            // Before paying: a session sent to build on an id the checkout
            // does not hold would look for it, then guess.
            return Err(Halt::Halted(format!(
                "task {} needs {}, which this checkout does not hold — the task that \
                 produces it has not merged, or the id is misspelled in `needs`",
                ctx.state.task.number,
                missing
                    .iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if !found.narrowed {
            ctx.traces.say(&format!(
                "tree: the whole checkout stays — {}",
                found.reason
            ));
            return Ok(Verdict::Continue);
        }
        // What an earlier session found missing, that the repository holds.
        let mut patterns = found.patterns.clone();
        let mut widened = Vec::new();
        for path in asked_for(&ctx.state.task.body) {
            let full = self.root.join(&path);
            if !self.disk.exists(&full) {
                continue;
            }
            let folder =
                !self.disk.dir_names(&full).is_empty() || !self.disk.file_names(&full).is_empty();
            patterns.push(if folder {
                format!("/{path}/")
            } else {
                format!("/{path}")
            });
            widened.push(path);
        }
        if !widened.is_empty() {
            ctx.traces.say(&format!(
                "tree: widened to what a session of this task found missing: {}",
                widened.join(", ")
            ));
        }
        let ran = self.repo.sparse_checkout(&patterns).await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot narrow the checkout to the task's closure: {}",
                ran.why()
            )));
        }
        let mut folders = closure::describe(&found);
        for path in &widened {
            let _ = write!(
                folders,
                "\n- /{path}/ (found missing by an earlier session)"
            );
        }
        ctx.traces.say(&format!(
            "tree: narrowed to the {} unit's closure ({} pattern(s), {} without a graph)",
            declaration.unit.key(),
            found.patterns.len(),
            if graph.is_some() {
                "not"
            } else {
                "narrowed by folder"
            }
        ));
        ctx.state.tree_note = folders;
        Ok(Verdict::Continue)
    }
}

/// Widens the checkout back to the whole tree.
pub struct Widen {
    /// The lane's checkout.
    pub repo: Rc<dyn Repo>,
}

#[async_trait(?Send)]
impl Action<Loop> for Widen {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.state.tree_note.is_empty() {
            return Ok(Verdict::Continue);
        }
        let ran = self.repo.sparse_checkout_off().await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot widen the checkout back to the whole tree: {}",
                ran.why()
            )));
        }
        ctx.state.tree_note = String::new();
        ctx.traces.say("tree: whole again");
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_process::FakeProcess;
    use harness_core::domain::Named;
    use harness_core::execution::Settings;
    use harness_core::ports::shell::git::Repos;
    use harness_core::traces::Logbook;
    use std::path::Path;

    fn context(body: &str) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            Loop {
                task: Named {
                    number: "34".to_string(),
                    title: "RiskBadge".to_string(),
                    body: body.to_string(),
                },
                ..Loop::default()
            },
            Logbook::null(),
        )
    }

    fn disk() -> FakeDisk {
        let mut disk = FakeDisk::default();
        for (path, content) in [
            ("/ws/contracts/README.md", "contracts"),
            (
                "/ws/crates/credit/capabilities/calculate-risk/capability.json",
                r#"{"capability":"credit.calculateRisk","system":"credit"}"#,
            ),
            (
                "/ws/apps/credit/risk-badge/micro-ui.json",
                r#"{"microUi":"RiskBadge","system":"credit"}"#,
            ),
            (
                "/ws/apps/credit/late-list/micro-ui.json",
                r#"{"microUi":"LatePaymentsList","system":"credit"}"#,
            ),
        ] {
            disk.existing
                .insert(PathBuf::from(path), content.to_string());
        }
        disk
    }

    fn narrow(repos: &Rc<FakeRepos>, process: FakeProcess) -> Narrow {
        Narrow {
            repo: repos.at(Path::new("/ws")),
            process: Rc::new(process),
            disk: Rc::new(disk()),
            root: PathBuf::from("/ws"),
        }
    }

    const MICRO_UI: &str = "## Architecture\n\nunit: micro-ui\nsystem: credit\nproduces: RiskBadge\nneeds: credit.calculateRisk\n";

    #[tokio::test]
    async fn a_declared_task_narrows_the_checkout_and_notes_what_is_there() {
        let repos = Rc::new(FakeRepos::default());
        let mut ctx = context(MICRO_UI);
        let verdict = narrow(
            &repos,
            FakeProcess::answering(vec![FakeProcess::ran("", "", 1)]),
        )
        .run(&mut ctx)
        .await
        .expect("narrowed");
        assert!(matches!(verdict, Verdict::Continue));
        assert!(repos.asked("sparse-checkout set"), "{:?}", repos.calls());
        assert!(
            ctx.state.tree_note.contains("- /apps/"),
            "{}",
            ctx.state.tree_note
        );
        assert!(ctx.state.tree_note.contains("- /mock/"));
        assert!(!ctx.state.tree_note.contains("late-list"));
    }

    #[tokio::test]
    async fn what_an_earlier_session_found_missing_widens_the_closure() {
        // ForgeDesMondes #31: the root workspace names crates/dataguard/server
        // and excludes tests/rules/business, which its closure left out.
        let repos = Rc::new(FakeRepos::default());
        let body = format!(
            "{MICRO_UI}\n## Decision\n\nblocks: AGENT_LOOP_STOP: missing apps/credit/late-list (named member) \
             and `contracts/README.md`, and nowhere/else: widen the closure (/code)\n"
        );
        let mut ctx = context(&body);
        narrow(
            &repos,
            FakeProcess::answering(vec![FakeProcess::ran("", "", 1)]),
        )
        .run(&mut ctx)
        .await
        .expect("narrowed");
        let mut plain = context(MICRO_UI);
        let other = Rc::new(FakeRepos::default());
        narrow(
            &other,
            FakeProcess::answering(vec![FakeProcess::ran("", "", 1)]),
        )
        .run(&mut plain)
        .await
        .expect("narrowed");
        let count = |calls: Vec<String>| {
            calls
                .iter()
                .find_map(|c| {
                    c.split_once("sparse-checkout set ")
                        .and_then(|(_, n)| n.parse::<usize>().ok())
                })
                .unwrap_or_default()
        };
        assert_eq!(
            count(repos.calls()),
            count(other.calls()) + 2,
            "two held paths added, the absent one not"
        );
        assert!(
            ctx.state
                .tree_note
                .contains("- /apps/credit/late-list/ (found missing"),
            "{}",
            ctx.state.tree_note
        );
        assert!(!ctx.state.tree_note.contains("nowhere/else"));
        assert_eq!(
            asked_for(
                "AGENT_LOOP_STOP: missing crates/dataguard/server (named workspace member; `cargo metadata` and gate data-capability-registered need it on disk) and tests/rules/business (acceptance criterion 5): widen the closure"
            ),
            vec![
                "crates/dataguard/server".to_string(),
                "tests/rules/business".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn a_task_declaring_nothing_leaves_the_tree_whole() {
        let repos = Rc::new(FakeRepos::default());
        let mut ctx = context("## Scope\n\nbranch: feat/x\n");
        narrow(&repos, FakeProcess::default())
            .run(&mut ctx)
            .await
            .expect("continues");
        assert!(!repos.asked("sparse-checkout"), "{:?}", repos.calls());
        assert_eq!(ctx.state.tree_note, "");
    }

    #[tokio::test]
    async fn tooling_leaves_the_tree_whole_and_says_why() {
        let repos = Rc::new(FakeRepos::default());
        let mut ctx = context("## Architecture\n\nunit: tooling\nsystem: platform\n");
        narrow(&repos, FakeProcess::default())
            .run(&mut ctx)
            .await
            .expect("continues");
        assert!(!repos.asked("sparse-checkout"));
    }

    #[tokio::test]
    async fn a_need_the_checkout_does_not_hold_halts_before_paying() {
        let repos = Rc::new(FakeRepos::default());
        let mut ctx = context(
            "## Architecture\n\nunit: micro-ui\nsystem: credit\nproduces: RiskBadge\nneeds: orders.list\n",
        );
        let err = narrow(&repos, FakeProcess::default())
            .run(&mut ctx)
            .await
            .expect_err("halts");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains("orders.list"), "{}", err.reason());
        assert!(!repos.asked("sparse-checkout"));
    }

    #[tokio::test]
    async fn a_need_a_delivered_sibling_produces_is_held_though_the_tree_cannot_index_it() {
        // ForgeDesMondes #32: `needs: database`, produced by #31's
        // infrastructure, merged into the milestone — no folder says so.
        let repos = Rc::new(FakeRepos::default());
        let mut ctx = context(
            "## Architecture\n\nunit: infrastructure\nsystem: platform\nneeds: database, orders.list\n",
        );
        ctx.state.delivered = vec!["Database".to_string()];
        let err = narrow(&repos, FakeProcess::default())
            .run(&mut ctx)
            .await
            .expect_err("orders.list is still missing");
        assert!(err.reason().contains("orders.list"), "{}", err.reason());
        assert!(!err.reason().contains("database"), "{}", err.reason());

        let mut ctx =
            context("## Architecture\n\nunit: infrastructure\nsystem: platform\nneeds: database\n");
        ctx.state.delivered = vec!["database".to_string()];
        let verdict = narrow(
            &repos,
            FakeProcess::answering(vec![FakeProcess::ran("", "", 1)]),
        )
        .run(&mut ctx)
        .await
        .expect("held");
        assert!(matches!(verdict, Verdict::Continue));
    }

    #[tokio::test]
    async fn a_git_that_cannot_narrow_fails_the_stage_rather_than_pretending() {
        let repos = Rc::new(FakeRepos::failing(&["sparse-checkout set"]));
        let mut ctx = context(MICRO_UI);
        let err = narrow(&repos, FakeProcess::default())
            .run(&mut ctx)
            .await
            .expect_err("fails");
        assert!(matches!(err, Halt::Failed(_)));
        assert_eq!(ctx.state.tree_note, "");
    }

    #[tokio::test]
    async fn the_graph_is_asked_of_cargo_metadata_in_the_checkout() {
        let repos = Rc::new(FakeRepos::default());
        let process = Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
            r#"{"workspace_root":"/ws","packages":[]}"#,
            "",
            0,
        )]));
        let action = Narrow {
            repo: repos.at(Path::new("/ws")),
            process: Rc::clone(&process) as Rc<dyn Process>,
            disk: Rc::new(disk()),
            root: PathBuf::from("/ws"),
        };
        let mut ctx = context(MICRO_UI);
        action.run(&mut ctx).await.expect("narrowed");
        let ran = process.ran.borrow();
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0].0, "cargo");
        assert_eq!(ran[0].1, ["metadata", "--format-version", "1", "--no-deps"]);
        assert_eq!(ran[0].2, Path::new("/ws"));
    }

    const CAPABILITY: &str =
        "## Architecture\n\nunit: capability\nsystem: credit\nproduces: credit.calculateRisk\n";

    #[tokio::test]
    async fn a_cargo_that_does_not_answer_narrows_by_folder() {
        let repos = Rc::new(FakeRepos::default());
        let mut ctx = context(CAPABILITY);
        narrow(
            &repos,
            FakeProcess::answering(vec![FakeProcess::ran("", "no workspace", 101)]),
        )
        .run(&mut ctx)
        .await
        .expect("narrowed");
        assert!(
            ctx.state.tree_note.contains("- /crates/credit/"),
            "{}",
            ctx.state.tree_note
        );
    }

    #[tokio::test]
    async fn a_cargo_that_answers_narrows_by_what_the_crate_reaches() {
        let repos = Rc::new(FakeRepos::default());
        let metadata = r#"{"workspace_root":"/ws","packages":[
            {"name":"calculate-risk","manifest_path":"/ws/crates/credit/capabilities/calculate-risk/Cargo.toml",
             "dependencies":[{"name":"credit-read","path":"/ws/crates/credit/read"}]},
            {"name":"credit-read","manifest_path":"/ws/crates/credit/read/Cargo.toml","dependencies":[]}
        ]}"#;
        let mut ctx = context(CAPABILITY);
        narrow(
            &repos,
            FakeProcess::answering(vec![FakeProcess::ran(metadata, "", 0)]),
        )
        .run(&mut ctx)
        .await
        .expect("narrowed");
        let note = &ctx.state.tree_note;
        assert!(note.contains("- /crates/credit/read/"), "{note}");
        assert!(
            !note.contains("- /crates/credit/\n"),
            "by the graph, not the folder: {note}"
        );
    }

    #[tokio::test]
    async fn widening_puts_the_tree_back_and_clears_the_note() {
        let repos = Rc::new(FakeRepos::default());
        let mut ctx = context(MICRO_UI);
        ctx.state.tree_note = "- /apps/".to_string();
        Widen {
            repo: repos.at(Path::new("/ws")),
        }
        .run(&mut ctx)
        .await
        .expect("widened");
        assert!(
            repos.asked("sparse-checkout disable"),
            "{:?}",
            repos.calls()
        );
        assert_eq!(ctx.state.tree_note, "");
    }

    #[tokio::test]
    async fn a_tree_that_was_never_narrowed_is_not_widened() {
        let repos = Rc::new(FakeRepos::default());
        let mut ctx = context(MICRO_UI);
        Widen {
            repo: repos.at(Path::new("/ws")),
        }
        .run(&mut ctx)
        .await
        .expect("nothing to do");
        assert!(!repos.asked("sparse-checkout"), "{:?}", repos.calls());
    }
}
