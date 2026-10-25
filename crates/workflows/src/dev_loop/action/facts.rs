//! The session facts written before the session opens: see
//! [`facts`](crate::dev_loop::data::facts).
//!
//! Unpaid and never a halt: a session without its facts runs its gates on
//! the base `gates-local` finds by itself and skips the closure check — the
//! loop judges the closure after it either way.
//!
//! After the session, [`SayGatesLocal`] says in the journal whether its last
//! `scripts/gates-local.sh` run judged the work it pushed, and whether that
//! run was partial (`--only`, `--failed`): the measure of whether sessions
//! run it, and of what it misses. It never gates anything. [`WriteFacts`]
//! removes the report an earlier round left, so a session that never ran
//! it reads "not run", never another round's verdict.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::process::Process;

use crate::dev_loop::data::facts::Facts;
use crate::dev_loop::data::state::Loop;

/// Writes the round's facts at `path`: see the module.
pub struct WriteFacts {
    /// What writes the file.
    pub disk: Rc<dyn Disk>,
    /// Where the session's environment says they are (`HARNESS_FACTS`);
    /// empty: nothing is written (a dry run, a test).
    pub path: PathBuf,
    /// The branch the task's pull request targets.
    pub integration_branch: String,
    /// The last `gates-local` report ([`GATES_LOCAL_REPORT`] under the
    /// checkout), removed before the session; empty: left alone.
    pub report: PathBuf,
}

#[async_trait(?Send)]
impl Action<Loop> for WriteFacts {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        if !self.report.as_os_str().is_empty()
            && let Err(why) = self.disk.remove_file(&self.report)
        {
            ctx.traces.warn(&format!(
                "an earlier gates-local report stays ({}) — the journal may read it as this round's",
                why.reason()
            ));
        }
        if self.path.as_os_str().is_empty() {
            return Ok(Verdict::Continue);
        }
        let facts = Facts::of(&ctx.state, &self.integration_branch);
        let written = serde_json::to_string_pretty(&facts)
            .map_err(|why| harness_core::domain::Halt::Failed(why.to_string()))
            .and_then(|text| {
                if let Some(parent) = self.path.parent() {
                    self.disk.create_dir_all(parent)?;
                }
                self.disk.write_to_string(&self.path, &text)
            });
        match written {
            Ok(()) => ctx.traces.debug(&format!(
                "session facts: #{} -> {} at {}",
                facts.task,
                facts.base,
                self.path.display()
            )),
            Err(why) => ctx.traces.warn(&format!(
                "session facts not written ({}) — gates-local finds its base itself and skips the closure check",
                why.reason()
            )),
        }
        Ok(Verdict::Continue)
    }
}

/// Where `gates-local` leaves its last report, under the checkout.
pub const GATES_LOCAL_REPORT: &str = ".git/gates-local/report.json";

/// What the journal says of a session's last `gates-local` run, against the
/// tree of the work it left (`HEAD^{tree}`): the report's `head_tree` is the
/// snapshot it judged.
#[must_use]
pub fn gates_local_line(report: Option<&str>, head_tree: &str) -> String {
    let Some(report) = report.and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
    else {
        return "gates-local: not run".to_string();
    };
    let judged = report["head_tree"].as_str().unwrap_or_default();
    let short = |sha: &str| sha.chars().take(7).collect::<String>();
    let count = |key: &str| report["counts"][key].as_u64().unwrap_or(0);
    let partial = report["partial"]
        .as_str()
        .map_or_else(String::new, |flags| format!(" (partial: {flags})"));
    let on = if !head_tree.is_empty() && judged == head_tree {
        format!("last run on the work pushed ({})", short(judged))
    } else {
        format!(
            "last run on other work than the one pushed ({} vs {})",
            short(judged),
            short(head_tree)
        )
    };
    format!(
        "gates-local: {on}{partial} — {} ok, {} FAIL, {} base-red, {} unreadable, closure {}",
        count("ok"),
        count("FAIL"),
        count("base-red"),
        count("unreadable"),
        report["closure"].as_str().unwrap_or("?")
    )
}

/// Says the session's last `gates-local` run in the journal: see the module.
pub struct SayGatesLocal {
    /// What reads the report.
    pub disk: Rc<dyn Disk>,
    /// What asks git the tree of the work left.
    pub process: Rc<dyn Process>,
    /// The checkout's root.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl Action<Loop> for SayGatesLocal {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let report = self
            .disk
            .read_to_string(&self.root.join(GATES_LOCAL_REPORT));
        let tree = self
            .process
            .run(
                "git",
                // `HEAD:` is the tree of HEAD, the work the session left.
                &["rev-parse".to_string(), "HEAD:".to_string()],
                &self.root,
                Duration::from_secs(30),
            )
            .await
            .map(|ran| ran.out().trim().to_string())
            .unwrap_or_default();
        ctx.traces.say(&gates_local_line(report.as_deref(), &tree));
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_disk::FakeDisk;
    use harness_core::domain::prompts::Named;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl harness_core::traces::Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn ctx(dry_run: bool) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            Loop {
                task: Named {
                    number: "7".to_string(),
                    title: "t".to_string(),
                    body: "branch: feat/t\n".to_string(),
                },
                ..Loop::default()
            },
            Logbook::null(),
        )
    }

    fn action(disk: &Rc<FakeDisk>, path: &str) -> WriteFacts {
        WriteFacts {
            disk: Rc::clone(disk) as Rc<dyn Disk>,
            path: PathBuf::from(path),
            integration_branch: "milestone/2-x".to_string(),
            report: PathBuf::from("/f/.git/gates-local/report.json"),
        }
    }

    #[tokio::test]
    async fn the_facts_are_written_where_the_session_reads_them() {
        let disk = Rc::new(FakeDisk::default());
        action(&disk, "/f/.llocal/loop/session-facts.json")
            .run(&mut ctx(false))
            .await
            .expect("runs");
        let text = disk
            .written_to(std::path::Path::new("/f/.llocal/loop/session-facts.json"))
            .expect("written");
        let facts = Facts::read(&text).expect("facts");
        assert_eq!(facts.task, "7");
        assert_eq!(facts.base, "origin/milestone/2-x");
        assert_eq!(facts.branch, "feat/t");
        assert_eq!(
            *disk.removed.borrow(),
            [PathBuf::from("/f/.git/gates-local/report.json")],
            "an earlier round's report never reads as this one's"
        );
    }

    #[test]
    fn the_journal_says_whether_the_last_run_judged_the_work_pushed() {
        assert_eq!(gates_local_line(None, "abc"), "gates-local: not run");
        assert_eq!(
            gates_local_line(Some("garbage"), "abc"),
            "gates-local: not run"
        );
        let report = r#"{"head_tree":"1234567890","counts":{"ok":30,"FAIL":1,"base-red":2,"unreadable":0},"closure":"ok"}"#;
        assert_eq!(
            gates_local_line(Some(report), "1234567890"),
            "gates-local: last run on the work pushed (1234567) — 30 ok, 1 FAIL, 2 base-red, 0 unreadable, closure ok"
        );
        assert!(gates_local_line(Some(report), "abcdef0123").starts_with(
            "gates-local: last run on other work than the one pushed (1234567 vs abcdef0)"
        ),);
        let partial =
            r#"{"head_tree":"1234567890","partial":"--failed","counts":{"ok":2},"closure":"skip"}"#;
        assert_eq!(
            gates_local_line(Some(partial), "1234567890"),
            "gates-local: last run on the work pushed (1234567) (partial: --failed) — 2 ok, 0 FAIL, 0 base-red, 0 unreadable, closure skip"
        );
    }

    #[tokio::test]
    async fn the_line_reads_the_report_and_the_tree_of_head() {
        use crate::common::fake_process::FakeProcess;
        use harness_core::ports::shell::process::Ran;
        let mut disk = FakeDisk::default();
        disk.existing.insert(
            PathBuf::from("/ws/.git/gates-local/report.json"),
            r#"{"head_tree":"feedbeef00","counts":{"ok":3},"closure":"skip"}"#.to_string(),
        );
        let process = Rc::new(FakeProcess::answering(vec![Ran {
            code: Some(0),
            stdout: "feedbeef00\n".to_string(),
            stderr: String::new(),
        }]));
        let lines = Rc::new(Capture::default());
        let mut context = ctx(false);
        context.traces = Logbook::new(
            Rc::clone(&lines) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        let said = || lines.0.borrow().join("\n");
        SayGatesLocal {
            disk: Rc::new(disk),
            process: Rc::clone(&process) as Rc<dyn Process>,
            root: PathBuf::from("/ws"),
        }
        .run(&mut context)
        .await
        .expect("runs");
        assert!(
            said().contains("last run on the work pushed (feedbee)"),
            "{}",
            said()
        );
        assert_eq!(process.ran.borrow()[0].1, ["rev-parse", "HEAD:"]);
    }

    #[tokio::test]
    async fn no_path_or_a_dry_run_writes_nothing() {
        let disk = Rc::new(FakeDisk::default());
        action(&disk, "").run(&mut ctx(false)).await.expect("runs");
        action(&disk, "/f/facts.json")
            .run(&mut ctx(true))
            .await
            .expect("runs");
        assert!(disk.written.borrow().is_empty());
        assert_eq!(
            disk.removed.borrow().len(),
            1,
            "the report goes without facts too, never on a dry run"
        );
    }
}
