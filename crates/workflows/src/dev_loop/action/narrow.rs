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
        if !found.missing.is_empty() {
            // Before paying: a session sent to build on an id the checkout
            // does not hold would look for it, then guess.
            return Err(Halt::Halted(format!(
                "task {} needs {}, which this checkout does not hold — the task that \
                 produces it has not merged, or the id is misspelled in `needs`",
                ctx.state.task.number,
                found.missing.join(", ")
            )));
        }
        if !found.narrowed {
            ctx.traces.say(&format!(
                "tree: the whole checkout stays — {}",
                found.reason
            ));
            return Ok(Verdict::Continue);
        }
        let ran = self.repo.sparse_checkout(&found.patterns).await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot narrow the checkout to the task's closure: {}",
                ran.why()
            )));
        }
        let folders = closure::describe(&found);
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
