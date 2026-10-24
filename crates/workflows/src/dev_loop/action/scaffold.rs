//! The unit's skeleton, written before the session opens.
//!
//! A session asked to build a `DataCapability` or a Capability spent its
//! first forty calls looking for a neighbour to copy — on the disk, then in
//! git's history once the tree was narrowed — and copied its drift along
//! with its shape. [`Scaffold`] writes the unit's folder from the harness's
//! templates instead (`harness_architecture::skeleton`), **uncommitted** in
//! the lane's checkout, before `Narrow`: the session's `git checkout -b`
//! carries the files to its branch, and the prompt tells it to start there.
//!
//! It never overwrites: an existing folder, a task branch already made (a
//! resumed or repaired task), a checkout with local changes — each is a
//! skip, said, and the session builds as it did before. Once written, the
//! registries are regenerated and the crate checked with its tests
//! (`cargo check --all-targets`); any step that fails takes everything back
//! — the folder removed, the generated files restored as they were — and
//! the round goes on without a skeleton. A failed skeleton is a template to
//! fix, never a task's.
//!
//! Through the ports only: `Disk` writes and reads, `Process` runs the
//! product's scripts and cargo, `Repo` answers whether the tree is clean.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use async_trait::async_trait;
use harness_architecture::skeleton::{self, REWRITTEN, Skeleton, Target};
use harness_architecture::tree::Tree;
use harness_architecture::vocabulary::{Declaration, Unit};
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::process::Process;

use crate::common::sections;
use crate::dev_loop::data::state::Loop;

/// How long a registry script may take.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the crate's check may take: a lane's first build of the day is
/// cold, and its time is won back by the session's own first build.
const CHECK_TIMEOUT: Duration = Duration::from_mins(15);

/// Writes the skeleton of the task's unit when there is none: see the module.
pub struct Scaffold {
    /// The lane's checkout.
    pub repo: Rc<dyn Repo>,
    /// What writes the files and reads the tree.
    pub disk: Rc<dyn Disk>,
    /// What runs the registries' scripts and cargo.
    pub process: Rc<dyn Process>,
    /// The checkout's root.
    pub root: PathBuf,
}

/// The task's declaration, as its body carries it.
fn declaration_of(body: &str) -> Option<Declaration> {
    sections::parse(body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
}

/// The last line a failed command printed, for the journal.
fn tail(text: &str) -> String {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_string()
}

/// What a rollback could not put back, said after its reason: nothing
/// when it put everything back.
fn not_put_back(left: &[String]) -> String {
    if left.is_empty() {
        String::new()
    } else {
        format!(" — and could not put back: {}", left.join("; "))
    }
}

impl Scaffold {
    /// Why no skeleton is written for `target`, when one is not: the
    /// never-overwrite rules.
    async fn blocked(&self, ctx: &Context<Loop>, target: &Target) -> Outcome<Option<String>> {
        if self.disk.exists(&self.root.join(&target.dir)) {
            return Ok(Some(format!("{} exists — never overwritten", target.dir)));
        }
        if let Some(query) = &target.query
            && self.disk.exists(&self.root.join(query))
        {
            return Ok(Some(format!("{query} exists — never overwritten")));
        }
        if let Some(branch) = crate::dev_loop::data::tasks::declared_branch(&ctx.state.task.body)
            && (self.repo.has_branch(&branch).await?
                || self.repo.origin_has_branch(&branch).await?)
        {
            return Ok(Some(format!(
                "the task's branch {branch} exists — a resumed or repaired task keeps its own files"
            )));
        }
        if !self.repo.dirty_files().await?.is_empty() {
            return Ok(Some(
                "the checkout carries local changes — nothing written beside them".to_string(),
            ));
        }
        Ok(None)
    }

    /// Whether the aggregate's crate in `dir` writes PostgreSQL tables.
    fn projects(&self, dir: &str) -> bool {
        let src = self.root.join(dir).join("src");
        self.disk.file_names(&src).iter().any(|name| {
            self.disk
                .read_to_string(&src.join(name))
                .is_some_and(|text| text.contains(".project_pg("))
        })
    }

    /// The skeleton of `target`, or why there is none.
    fn rendered(&self, target: &Target, declaration: &Declaration) -> Result<Skeleton, String> {
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        if !tree.under_architecture {
            return Err("no contracts/ — not under the architecture".to_string());
        }
        let aggregate = declaration
            .aggregate
            .as_deref()
            .filter(|_| target.unit == Unit::DataCapability)
            .and_then(|name| skeleton::aggregate_crate(&tree, name));
        let projects = aggregate.as_ref().is_some_and(|a| self.projects(&a.dir));
        skeleton::render(target, declaration, &tree, aggregate.as_ref(), projects)
            .map_err(|skip| skip.to_string())
    }

    /// Writes the files, then runs the commands; the first failure, said.
    async fn write_and_check(&self, skeleton: &Skeleton) -> Result<(), String> {
        for (path, text) in &skeleton.files {
            let full = self.root.join(path);
            if let Some(parent) = full.parent() {
                self.disk
                    .create_dir_all(parent)
                    .map_err(|e| format!("{path}: {e}"))?;
            }
            self.disk
                .write_to_string(&full, text)
                .map_err(|e| format!("{path}: {e}"))?;
        }
        for command in &skeleton.commands {
            let Some((program, args)) = command.split_first() else {
                continue;
            };
            let timeout = if program == "cargo" {
                CHECK_TIMEOUT
            } else {
                SCRIPT_TIMEOUT
            };
            let said = command.join(" ");
            match self.process.run(program, args, &self.root, timeout).await {
                Ok(ran) if ran.ok() => {}
                Ok(ran) => {
                    return Err(format!(
                        "`{said}` failed: {}",
                        tail(&format!("{}\n{}", ran.stdout, ran.stderr))
                    ));
                }
                Err(e) => return Err(format!("`{said}` did not run: {e}")),
            }
        }
        Ok(())
    }

    /// Takes the skeleton back: its folder and its query file removed, the
    /// generated files as they were before. What could not be put back,
    /// said.
    fn roll_back(&self, target: &Target, before: &[(&str, Option<String>)]) -> Vec<String> {
        let mut left = Vec::new();
        if let Err(e) = self.disk.remove_dir_all(&self.root.join(&target.dir)) {
            left.push(format!("{}: {e}", target.dir));
        }
        if let Some(query) = &target.query
            && let Err(e) = self.disk.remove_file(&self.root.join(query))
        {
            left.push(format!("{query}: {e}"));
        }
        for (path, text) in before {
            let full = self.root.join(path);
            // A file that was not there is removed: absent already is fine.
            let put = text.as_ref().map_or_else(
                || self.disk.remove_file(&full),
                |text| self.disk.write_to_string(&full, text),
            );
            if let Err(e) = put {
                left.push(format!("{path}: {e}"));
            }
        }
        left
    }
}

#[async_trait(?Send)]
impl Action<Loop> for Scaffold {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        ctx.state.skeleton = Vec::new();
        let Some(declaration) = declaration_of(&ctx.state.task.body) else {
            return Ok(Verdict::Continue);
        };
        if !matches!(declaration.unit, Unit::DataCapability | Unit::Capability) {
            return Ok(Verdict::Continue);
        }
        let target = match skeleton::target(&declaration) {
            Ok(target) => target,
            Err(skip) => {
                ctx.traces.say(&format!("skeleton: none — {skip}"));
                return Ok(Verdict::Continue);
            }
        };
        if let Some(why) = self.blocked(ctx, &target).await? {
            ctx.traces.say(&format!("skeleton: none — {why}"));
            return Ok(Verdict::Continue);
        }
        let skeleton = match self.rendered(&target, &declaration) {
            Ok(skeleton) => skeleton,
            Err(why) => {
                ctx.traces.say(&format!("skeleton: none — {why}"));
                return Ok(Verdict::Continue);
            }
        };
        let paths: Vec<String> = skeleton
            .files
            .iter()
            .map(|(path, _)| path.clone())
            .collect();
        if ctx.settings.dry_run {
            ctx.traces.say(&format!(
                "skeleton (dry-run): would write {}",
                paths.join(", ")
            ));
            return Ok(Verdict::Continue);
        }
        let before: Vec<(&str, Option<String>)> = REWRITTEN
            .iter()
            .map(|path| (*path, self.disk.read_to_string(&self.root.join(path))))
            .collect();
        if let Err(why) = self.write_and_check(&skeleton).await {
            let left = self.roll_back(&target, &before);
            ctx.traces.warn(&format!(
                "skeleton: {} taken back, the session builds without one — {why}{}",
                target.id,
                not_put_back(&left)
            ));
            return Ok(Verdict::Continue);
        }
        ctx.traces.say(&format!(
            "skeleton: {} written uncommitted (templates v{}), {} file(s), {} hole(s) to fill",
            target.id,
            skeleton::VERSION,
            paths.len(),
            skeleton.holes()
        ));
        ctx.state.skeleton = paths;
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
    use harness_core::ports::shell::git::Repos as _;
    use harness_core::traces::Logbook;
    use std::path::Path;

    const DATA_CAPABILITY: &str = "## Scope\n\nbranch: feat/move-token\n\n## Architecture\n\nunit: data-capability\nsystem: table\neffect: update\ntouches: q\naggregate: Token\nproduces: table.moveToken\n";
    const CAPABILITY: &str =
        "## Architecture\n\nunit: capability\nsystem: table\nproduces: table.listTokens\n";

    fn context(body: &str, dry_run: bool) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            Loop {
                task: Named {
                    number: "61".to_string(),
                    title: "moveToken".to_string(),
                    body: body.to_string(),
                },
                skeleton: vec!["stale".to_string()],
                ..Loop::default()
            },
            Logbook::null(),
        )
    }

    /// A checkout under the architecture holding the aggregate `Token`.
    fn disk(extra: &[(&str, &str)]) -> FakeDisk {
        let mut disk = FakeDisk::default();
        for (path, content) in [
            ("/ws/contracts/README.md", "contracts"),
            (
                "/ws/crates/table/aggregates/Token/aggregate.json",
                r#"{"aggregate":"Token"}"#,
            ),
            (
                "/ws/crates/table/aggregates/Token/Cargo.toml",
                "[package]\nname = \"aggregate-token\"\n",
            ),
            (
                "/ws/crates/table/aggregates/Token/src/lib.rs",
                "Plugin::new().project_pg(\"Token\", Tables)",
            ),
            ("/ws/Cargo.lock", "the lock before"),
        ]
        .into_iter()
        .chain(extra.iter().copied())
        {
            disk.existing
                .insert(PathBuf::from(path), content.to_string());
        }
        disk
    }

    /// A `git` that holds no branch of the task.
    fn fresh() -> FakeRepos {
        FakeRepos::default().with_branches(&[], &[])
    }

    struct Run {
        disk: Rc<FakeDisk>,
        process: Rc<FakeProcess>,
        ctx: Context<Loop>,
    }

    async fn scaffold(
        body: &str,
        disk: FakeDisk,
        repos: &FakeRepos,
        answers: Vec<harness_core::ports::shell::process::Ran>,
        dry_run: bool,
    ) -> Run {
        let disk = Rc::new(disk);
        let process = Rc::new(FakeProcess::answering(answers));
        let mut ctx = context(body, dry_run);
        let verdict = Scaffold {
            repo: repos.at(Path::new("/ws")),
            disk: Rc::clone(&disk) as Rc<dyn Disk>,
            process: Rc::clone(&process) as Rc<dyn Process>,
            root: PathBuf::from("/ws"),
        }
        .run(&mut ctx)
        .await
        .expect("never stops the round");
        assert!(matches!(verdict, Verdict::Continue));
        Run { disk, process, ctx }
    }

    fn written(run: &Run) -> Vec<String> {
        run.disk
            .written
            .borrow()
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect()
    }

    #[tokio::test]
    async fn a_data_capability_s_skeleton_is_written_registered_and_checked() {
        let run = scaffold(DATA_CAPABILITY, disk(&[]), &fresh(), Vec::new(), false).await;
        let unit = "crates/table/data-capabilities/move-token";
        assert_eq!(
            run.ctx.state.skeleton,
            [
                format!("{unit}/Cargo.toml"),
                format!("{unit}/data-capability.json"),
                format!("{unit}/src/lib.rs"),
                format!("{unit}/tests/prepare.rs"),
                format!("{unit}/tests/engine.rs"),
                format!("{unit}/tests/postgres.rs"),
            ],
            "the aggregate projects: a PostgreSQL frame too"
        );
        assert!(
            written(&run)
                .iter()
                .all(|p| p.starts_with(&format!("/ws/{unit}/")))
        );
        let ran = run.process.ran.borrow();
        let commands: Vec<String> = ran
            .iter()
            .map(|(program, args, cwd)| {
                assert_eq!(cwd, Path::new("/ws"));
                format!("{program} {}", args.join(" "))
            })
            .collect();
        assert_eq!(
            commands,
            [
                "bash scripts/register.sh",
                "cargo check --quiet -p move-token --all-targets"
            ]
        );
        assert_eq!(
            *run.process.timeouts.borrow(),
            [SCRIPT_TIMEOUT, CHECK_TIMEOUT],
            "the check gets the long clock"
        );
        assert_eq!(run.disk.removed.borrow().len(), 0, "nothing taken back");
    }

    #[tokio::test]
    async fn a_capability_s_skeleton_writes_its_query_and_registers_it() {
        let run = scaffold(CAPABILITY, disk(&[]), &fresh(), Vec::new(), false).await;
        assert!(
            run.ctx
                .state
                .skeleton
                .contains(&"crates/table/queries/list-tokens.graphql".to_string())
        );
        let ran = run.process.ran.borrow();
        assert_eq!(ran[1].1, ["scripts/queries.sh"]);
        assert_eq!(ran.len(), 3);
    }

    #[tokio::test]
    async fn a_failing_check_takes_everything_back_and_the_round_goes_on() {
        let repos = FakeRepos::default();
        let run = scaffold(
            CAPABILITY,
            disk(&[]),
            &repos,
            vec![
                FakeProcess::ran("registered", "", 0),
                FakeProcess::ran("", "", 0),
                FakeProcess::ran("", "error[E0425]: cannot find value\nmore", 101),
            ],
            false,
        )
        .await;
        assert_eq!(run.ctx.state.skeleton.len(), 0);
        let removed: Vec<String> = run
            .disk
            .removed
            .borrow()
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        assert!(
            removed.contains(&"/ws/crates/table/capabilities/list-tokens".to_string()),
            "{removed:?}"
        );
        assert!(
            removed.contains(&"/ws/crates/table/queries/list-tokens.graphql".to_string()),
            "{removed:?}"
        );
        assert_eq!(
            run.disk.written_to(Path::new("/ws/Cargo.lock")).as_deref(),
            Some("the lock before"),
            "the lock as it was"
        );
        assert!(
            !run.disk
                .written
                .borrow()
                .iter()
                .any(|(p, _)| p == Path::new("/ws/queries/registry.json")),
            "a file that was not there is not written back"
        );
        assert!(
            removed.contains(&"/ws/queries/registry.json".to_string()),
            "a file that was not there is removed: {removed:?}"
        );
        assert!(
            !removed.contains(&"/ws/Cargo.lock".to_string()),
            "{removed:?}"
        );
        assert_eq!(not_put_back(&[]), "");
        assert_eq!(
            not_put_back(&["a: no".to_string(), "b: no".to_string()]),
            " — and could not put back: a: no; b: no"
        );
    }

    #[tokio::test]
    async fn a_script_that_fails_or_does_not_run_takes_the_skeleton_back_before_the_check() {
        let run = scaffold(
            DATA_CAPABILITY,
            disk(&[]),
            &fresh(),
            vec![FakeProcess::ran("", "a unit the registry does not name", 1)],
            false,
        )
        .await;
        assert_eq!(run.ctx.state.skeleton.len(), 0);
        assert_eq!(
            run.process.ran.borrow().len(),
            1,
            "no check after a failed script"
        );
        assert!(!run.disk.removed.borrow().is_empty());
        assert_eq!(tail("a\n\n  b  \n\n"), "b");
        assert_eq!(tail(""), "no output");
    }

    #[tokio::test]
    async fn an_existing_unit_or_query_is_never_overwritten() {
        for extra in [
            (
                "/ws/crates/table/data-capabilities/move-token/Cargo.toml",
                "x",
            ),
            ("/ws/crates/table/queries/list-tokens.graphql", "x"),
        ] {
            for body in [DATA_CAPABILITY, CAPABILITY] {
                let run = scaffold(body, disk(&[extra]), &fresh(), Vec::new(), false).await;
                let blocked = (body == DATA_CAPABILITY) == extra.0.contains("move-token");
                assert_eq!(
                    run.ctx.state.skeleton.is_empty(),
                    blocked,
                    "{body} with {}",
                    extra.0
                );
                if blocked {
                    assert_eq!(written(&run).len(), 0);
                    assert_eq!(run.process.ran.borrow().len(), 0);
                }
            }
        }
    }

    #[tokio::test]
    async fn a_task_branch_already_made_or_a_dirty_checkout_is_left_alone() {
        let local = FakeRepos::default().with_branches(&["feat/move-token"], &[]);
        let run = scaffold(DATA_CAPABILITY, disk(&[]), &local, Vec::new(), false).await;
        assert_eq!(written(&run).len(), 0);
        let remote = FakeRepos::default().with_branches(&[], &["feat/move-token"]);
        let run = scaffold(DATA_CAPABILITY, disk(&[]), &remote, Vec::new(), false).await;
        assert_eq!(written(&run).len(), 0);
        let dirty = fresh().rewriting(&["Cargo.toml"]);
        let run = scaffold(DATA_CAPABILITY, disk(&[]), &dirty, Vec::new(), false).await;
        assert_eq!(written(&run).len(), 0);
        assert_eq!(run.ctx.state.skeleton.len(), 0, "a stale list is cleared");
    }

    #[tokio::test]
    async fn what_the_templates_do_not_cover_writes_nothing() {
        for body in [
            "## Scope\n\nnothing declared\n",
            "## Architecture\n\nunit: micro-ui\nsystem: table\nproduces: Battlemap\n",
            "## Architecture\n\nunit: capability\nsystem: table\nhost: hostless\nproduces: table.listTokens\n",
            "## Architecture\n\nunit: data-capability\nsystem: table\naggregate: Ghost\nproduces: table.moveToken\n",
        ] {
            let run = scaffold(body, disk(&[]), &fresh(), Vec::new(), false).await;
            assert_eq!(written(&run).len(), 0, "{body}");
            assert_eq!(run.ctx.state.skeleton.len(), 0, "{body}");
        }
        let mut outside = FakeDisk::default();
        outside
            .existing
            .insert(PathBuf::from("/ws/Cargo.lock"), String::new());
        let run = scaffold(CAPABILITY, outside, &fresh(), Vec::new(), false).await;
        assert_eq!(
            written(&run).len(),
            0,
            "no contracts/: not under the architecture"
        );
    }

    #[tokio::test]
    async fn a_dry_run_says_what_it_would_write_and_writes_nothing() {
        let run = scaffold(CAPABILITY, disk(&[]), &fresh(), Vec::new(), true).await;
        assert_eq!(written(&run).len(), 0);
        assert_eq!(run.process.ran.borrow().len(), 0);
        assert_eq!(run.ctx.state.skeleton.len(), 0);
    }

    #[tokio::test]
    async fn an_aggregate_that_projects_nothing_gets_no_postgres_frame() {
        let mut quiet = disk(&[]);
        quiet.existing.insert(
            PathBuf::from("/ws/crates/table/aggregates/Token/src/lib.rs"),
            "Plugin::new().judge(\"Token\", Rules)".to_string(),
        );
        let run = scaffold(DATA_CAPABILITY, quiet, &fresh(), Vec::new(), false).await;
        assert_eq!(run.ctx.state.skeleton.len(), 5);
        assert!(
            !run.ctx
                .state
                .skeleton
                .iter()
                .any(|p| p.ends_with("postgres.rs"))
        );
    }
}
