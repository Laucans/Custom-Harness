//! Assembles one draft from ports and a config already built — the concrete
//! adapters stay the launcher's.

use std::cell::Cell;

use crate::data_model_draft::config::Config;
use crate::data_model_draft::orchestration::round;
use crate::data_model_draft::orchestration::workflow::DraftRun;
use crate::data_model_draft::ports::Ports;

/// Assembles one draft.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> DraftRun {
    DraftRun {
        remaining: Cell::new(1),
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    use harness_core::domain::{Halt, Slug};
    use harness_core::execution::{Action, Context, Settings};
    use harness_core::ports::agent::SessionSpec;
    use harness_core::traces::{Logbook, Sink, Verbosity};

    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::common::fake_process::FakeProcess;
    use crate::data_model_draft::action::prepare::Prepare;
    use crate::data_model_draft::action::publish::Publish;
    use crate::data_model_draft::config::Config;
    use crate::data_model_draft::data::state::DraftState;
    use crate::data_model_draft::ports::Ports;
    use crate::pr_review::ports::fake::{NoSessions, Nowhere};

    const CLONE: &str = "/tmp/draft/o-r";

    struct Quiet;

    impl Sink for Quiet {
        fn emit(&self, _line: &str) {}
    }

    fn ctx() -> Context<DraftState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            DraftState::default(),
            Logbook::new(Rc::new(Quiet), Verbosity::Normal),
        )
    }

    fn now() -> String {
        "2026-10-09T12:00:00Z".to_string()
    }

    fn wired(
        gh: Rc<FakeGitHub>,
        repos: FakeRepos,
        disk: Rc<FakeDisk>,
        process: Rc<FakeProcess>,
    ) -> Ports {
        Ports {
            gh,
            repos: Rc::new(repos),
            disk,
            process,
            sessions: Rc::new(NoSessions),
            spending: Rc::new(Nowhere),
            now,
        }
    }

    fn config() -> Config {
        Config {
            slug: Slug {
                owner: "o".to_string(),
                name: "r".to_string(),
            },
            base: "main_agent".to_string(),
            workdir: PathBuf::from("/tmp/draft"),
            script: "scripts/dump-schema.sh".to_string(),
            timeout: Duration::from_secs(60),
            spec: SessionSpec {
                model: "opus".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    fn github() -> Rc<FakeGitHub> {
        Rc::new(FakeGitHub {
            branch_shas: HashMap::from([(
                "main_agent".to_string(),
                Some("abcdef0123".to_string()),
            )]),
            ..FakeGitHub::default()
        })
    }

    #[test]
    fn the_draft_prepares_drafts_then_publishes() {
        use crate::data_model_draft::orchestration::stages::{self, DRAFT, PREPARE, PUBLISH};
        let ports = wired(
            github(),
            FakeRepos::default(),
            Rc::new(FakeDisk::default()),
            Rc::new(FakeProcess::answering(Vec::new())),
        );
        let names: Vec<String> = stages::table(&ports, &config())
            .into_iter()
            .map(|stage| stage.name)
            .collect();
        assert_eq!(names, [PREPARE, DRAFT, PUBLISH]);
    }

    #[tokio::test]
    async fn the_head_is_dumped_and_a_draft_branch_made_from_it() {
        let mut disk = FakeDisk::default();
        disk.existing
            .insert(PathBuf::from(format!("{CLONE}/README.md")), String::new());
        let disk = Rc::new(disk);
        let repos = FakeRepos::default();
        let ports = wired(
            github(),
            repos.clone(),
            Rc::clone(&disk),
            Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
                "CREATE TABLE t (a int);\n",
                "",
                0,
            )])),
        );
        let mut ctx = ctx();
        Prepare::new(&ports, &config())
            .run(&mut ctx)
            .await
            .expect("prepared");
        assert_eq!(ctx.state.branch, "harness/model-draft/main_agent");
        assert_eq!(ctx.state.sha, "abcdef0123");
        assert_eq!(
            disk.written_to(&PathBuf::from("/tmp/draft/main_agent.schema.sql"))
                .as_deref(),
            Some("CREATE TABLE t (a int);\n")
        );
        assert!(repos.asked("create_local_branch harness/model-draft/main_agent abcdef0123"));
    }

    #[tokio::test]
    async fn a_branch_that_has_a_model_or_no_schema_is_not_drafted() {
        let mut disk = FakeDisk::default();
        disk.existing.insert(
            PathBuf::from(format!("{CLONE}/data/model.json")),
            "{}".to_string(),
        );
        let ports = wired(
            github(),
            FakeRepos::default(),
            Rc::new(disk),
            Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
                "CREATE TABLE t (a int);",
                "",
                0,
            )])),
        );
        let refused = Prepare::new(&ports, &config())
            .run(&mut ctx())
            .await
            .expect_err("refused");
        assert!(
            matches!(&refused, Halt::Halted(why) if why.contains("already has")),
            "{refused:?}"
        );

        let mut disk = FakeDisk::default();
        disk.existing
            .insert(PathBuf::from(format!("{CLONE}/x")), String::new());
        let ports = wired(
            github(),
            FakeRepos::default(),
            Rc::new(disk),
            Rc::new(FakeProcess::default()),
        );
        let refused = Prepare::new(&ports, &config())
            .run(&mut ctx())
            .await
            .expect_err("refused");
        assert!(
            matches!(&refused, Halt::Halted(why) if why.contains("printed nothing")),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn the_drafted_model_is_checked_committed_and_proposed() {
        let model = r#"{"version":1,"tables":[{"name":"t","purpose":"p","fields":[{"name":"a","type":"integer","purpose":"q"}]}]}"#;
        let mut disk = FakeDisk::default();
        disk.existing.insert(
            PathBuf::from(format!("{CLONE}/data/model.json")),
            model.to_string(),
        );
        let gh = github();
        let repos = FakeRepos::default();
        let publish = Publish {
            gh: Rc::clone(&gh) as Rc<dyn harness_core::ports::shell::github::GitHub>,
            repos: Rc::new(repos.clone()),
            disk: Rc::new(disk),
            base: "main_agent".to_string(),
        };
        let mut ctx = ctx();
        ctx.state.clone = PathBuf::from(CLONE);
        ctx.state.branch = "harness/model-draft/main_agent".to_string();
        ctx.state.sha = "abcdef0123".to_string();
        ctx.state.schema = "CREATE TABLE t (a integer);".to_string();
        publish.run(&mut ctx).await.expect("published");
        assert_eq!(
            ctx.state.pr.as_deref(),
            Some("https://github.com/owner/repo/pull/1")
        );
        assert!(
            ctx.state.divergences.is_empty(),
            "nullability the model leaves unsaid is no divergence: {:?}",
            ctx.state.divergences
        );
        assert!(repos.asked("commit docs(data): a first data/model.json for main_agent"));
        assert!(repos.asked("push harness/model-draft/main_agent"));
        assert!(gh.wrote.borrow().iter().any(|w| matches!(w, Wrote::CreatedPr(head, base, _) if head == "harness/model-draft/main_agent" && base == "main_agent")));
    }

    #[tokio::test]
    async fn a_model_that_is_not_json_is_not_published() {
        let mut disk = FakeDisk::default();
        disk.existing.insert(
            PathBuf::from(format!("{CLONE}/data/model.json")),
            "not json".to_string(),
        );
        let repos = FakeRepos::default();
        let publish = Publish {
            gh: github() as Rc<dyn harness_core::ports::shell::github::GitHub>,
            repos: Rc::new(repos.clone()),
            disk: Rc::new(disk),
            base: "main_agent".to_string(),
        };
        let mut ctx = ctx();
        ctx.state.clone = PathBuf::from(CLONE);
        let refused = publish.run(&mut ctx).await.expect_err("refused");
        assert!(
            matches!(&refused, Halt::Failed(why) if why.contains("not the model's JSON")),
            "{refused:?}"
        );
        assert!(!repos.asked("push"), "nothing pushed");
    }
}
