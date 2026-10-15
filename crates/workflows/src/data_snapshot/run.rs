//! Assembles one data snapshot from ports and a config already built.
//!
//! **Not the construction of concrete adapters** — that stays the
//! launcher's work. No `Context`, no stage table: a deterministic command
//! (see `mod.rs`).

use harness_core::domain::Outcome;

use crate::data_snapshot::action::archive::{INDEX, archive};
use crate::data_snapshot::action::capture::{capture, mount};
use crate::data_snapshot::config::Config;
use crate::data_snapshot::data::index::Index;
use crate::data_snapshot::data::report::{Report, Snapped};
use crate::data_snapshot::ports::Ports;

/// What an invocation requests.
pub struct Request {
    /// The branches whose head is read: `main`, the integration branch, the
    /// open milestones' branches.
    pub branches: Vec<String>,
}

/// One data snapshot, assembled from its wiring.
pub struct DataSnapshotRun<'a> {
    ports: &'a Ports,
    config: &'a Config,
    branches: Vec<String>,
}

/// Assembles one data snapshot.
#[must_use]
pub fn build<'a>(ports: &'a Ports, config: &'a Config, request: Request) -> DataSnapshotRun<'a> {
    DataSnapshotRun {
        ports,
        config,
        branches: request.branches,
    }
}

impl DataSnapshotRun<'_> {
    /// Reads every head that moved, archives what changed.
    ///
    /// Nothing is cloned when no head moved: one GitHub read per branch, and
    /// the index, is the whole cost of a quiet pass.
    ///
    /// # Errors
    ///
    /// GitHub, `git`, the disk or the script could not be reached at all. A
    /// refusal — a script that fails, a push rejected — is a
    /// [`Snapped::Failed`] in the report, not an error: the other branches
    /// are still read.
    pub async fn run(&self) -> Outcome<Report> {
        let (ports, config) = (self.ports, self.config);
        let kept = Index::parse(
            ports
                .gh
                .file_text(INDEX, &config.archive_branch)
                .await?
                .as_deref(),
        );
        let mut heads = Vec::new();
        for branch in &self.branches {
            heads.push((branch.clone(), ports.gh.branch_sha(branch).await?));
        }
        let pending = kept.pending(&heads);
        if pending.is_empty() {
            return Ok(Report::default());
        }
        let failed = |why: &str| -> Report {
            Report {
                snapped: pending
                    .iter()
                    .map(|(branch, sha)| Snapped::Failed {
                        branch: branch.clone(),
                        sha: sha.clone(),
                        reason: why.to_string(),
                    })
                    .collect(),
            }
        };
        let clone = match mount(ports, config).await? {
            Ok(clone) => clone,
            Err(why) => return Ok(failed(&why)),
        };
        let mut report = Report::default();
        let mut captured = Vec::new();
        for head in &pending {
            match capture(ports, config, &clone, head).await? {
                Ok(c) => captured.push(c),
                Err(reason) => report.snapped.push(Snapped::Failed {
                    branch: head.0.clone(),
                    sha: head.1.clone(),
                    reason,
                }),
            }
        }
        if captured.is_empty() {
            return Ok(report);
        }
        match archive(ports, config, &clone, &captured).await? {
            Ok(snapped) => report.snapped.extend(snapped),
            Err(why) => report
                .snapped
                .extend(captured.iter().map(|c| Snapped::Failed {
                    branch: c.branch.clone(),
                    sha: c.sha.clone(),
                    reason: why.clone(),
                })),
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::rc::Rc;

    use super::*;
    use crate::common::fake_disk::FakeDisk;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::FakeGitHub;
    use crate::common::fake_process::FakeProcess;
    use crate::data_snapshot::config::fake::config;
    use crate::data_snapshot::ports::fake;

    const CLONE: &str = "/tmp/snap/o-r";

    fn github(heads: &[(&str, Option<&str>)], index: Option<&str>) -> Rc<FakeGitHub> {
        let gh = FakeGitHub {
            branch_shas: heads
                .iter()
                .map(|(b, s)| ((*b).to_string(), s.map(str::to_string)))
                .collect(),
            files: index
                .map(|index| {
                    HashMap::from([(
                        ("index.json".to_string(), "harness/data-model".to_string()),
                        index.to_string(),
                    )])
                })
                .unwrap_or_default(),
            ..FakeGitHub::default()
        };
        Rc::new(gh)
    }

    fn request() -> Request {
        Request {
            branches: vec!["main".into(), "main_agent".into(), "milestone/3-x".into()],
        }
    }

    #[tokio::test]
    async fn a_quiet_pass_clones_nothing() {
        let index = r#"{"version":1,"heads":{"main":"a","main_agent":"b"},"snapshots":[]}"#;
        let repos = FakeRepos::default();
        let process = Rc::new(FakeProcess::default());
        let ports = fake::with(
            github(
                &[
                    ("main", Some("a")),
                    ("main_agent", Some("b")),
                    ("milestone/3-x", None),
                ],
                Some(index),
            ),
            repos.clone(),
            Rc::new(FakeDisk::default()),
            Rc::clone(&process),
        );
        let config = config();
        let report = build(&ports, &config, request()).run().await.expect("ran");
        assert_eq!(report, Report::default());
        assert!(repos.calls().is_empty(), "{:?}", repos.calls());
        assert!(process.ran.borrow().is_empty(), "expected nothing");
    }

    #[tokio::test]
    async fn a_moved_head_is_dumped_and_archived_on_a_new_orphan_branch() {
        let repos = FakeRepos::default();
        let process = Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
            "CREATE TABLE t (a int);\n",
            "",
            0,
        )]));
        let mut disk = FakeDisk::default();
        // The clone exists already; the target keeps no script of its own.
        disk.existing
            .insert(PathBuf::from(format!("{CLONE}/README.md")), String::new());
        disk.existing.insert(
            PathBuf::from(format!("{CLONE}/data/model.json")),
            "{\"tables\":[]}".to_string(),
        );
        let disk = Rc::new(disk);
        let ports = fake::with(
            github(
                &[
                    ("main", Some("abcdef0123")),
                    ("main_agent", None),
                    ("milestone/3-x", None),
                ],
                None,
            ),
            repos.clone(),
            Rc::clone(&disk),
            Rc::clone(&process),
        );
        let config = config();
        let report = build(&ports, &config, request()).run().await.expect("ran");
        let dir = "main/20261009T120000Z-abcdef0";
        assert_eq!(
            report.snapped,
            [Snapped::Archived {
                branch: "main".into(),
                sha: "abcdef0123".into(),
                dir: dir.into(),
                compared: true,
                divergences: vec!["table t is in the database, not in the model".into()],
            }]
        );
        // The default script, run from the clone.
        let ran = process.ran.borrow();
        assert_eq!(ran[0].1, ["/tmp/snap/dump-schema.sh"]);
        assert_eq!(ran[0].2, PathBuf::from(CLONE));
        assert!(
            disk.written_to(&PathBuf::from("/tmp/snap/dump-schema.sh"))
                .is_some_and(|s| s.contains("pg_dump --schema-only"))
        );
        assert_eq!(
            disk.written_to(&PathBuf::from(format!("{CLONE}/{dir}/schema.sql")))
                .as_deref(),
            Some("CREATE TABLE t (a int);\n")
        );
        assert!(
            disk.written_to(&PathBuf::from(format!("{CLONE}/{dir}/model.json")))
                .is_some()
        );
        let index = Index::parse(
            disk.written_to(&PathBuf::from(format!("{CLONE}/index.json")))
                .as_deref(),
        );
        assert_eq!(index.heads["main"], "abcdef0123");
        assert_eq!(index.snapshots.len(), 1);
        // The fake says origin has every branch: checked out, reset, never orphaned.
        assert!(repos.asked("reset_hard abcdef0123"));
        assert!(repos.asked("checkout harness/data-model --force"));
        assert!(repos.asked("commit data snapshot: main @ abcdef0"));
        assert!(repos.asked("push harness/data-model"));
    }

    #[tokio::test]
    async fn an_unchanged_schema_moves_the_head_and_adds_no_folder() {
        let dir = "main/20261001T000000Z-0000000";
        let index = format!(
            r#"{{"version":1,"heads":{{"main":"old"}},"snapshots":[{{"branch":"main","sha":"old","at":"2026-10-01T00:00:00Z","dir":"{dir}","schema":true,"model":false}}]}}"#
        );
        let mut disk = FakeDisk::default();
        disk.existing
            .insert(PathBuf::from(format!("{CLONE}/index.json")), index.clone());
        disk.existing.insert(
            PathBuf::from(format!("{CLONE}/{dir}/schema.sql")),
            "S".into(),
        );
        let disk = Rc::new(disk);
        let ports = fake::with(
            github(&[("main", Some("new"))], Some(&index)),
            FakeRepos::default(),
            Rc::clone(&disk),
            Rc::new(FakeProcess::answering(vec![FakeProcess::ran("S", "", 0)])),
        );
        let config = config();
        let report = build(
            &ports,
            &config,
            Request {
                branches: vec!["main".into()],
            },
        )
        .run()
        .await
        .expect("ran");
        assert_eq!(
            report.snapped,
            [Snapped::Unchanged {
                branch: "main".into(),
                sha: "new".into()
            }]
        );
        let kept = Index::parse(
            disk.written_to(&PathBuf::from(format!("{CLONE}/index.json")))
                .as_deref(),
        );
        assert_eq!(kept.heads["main"], "new");
        assert_eq!(kept.snapshots.len(), 1);
    }

    #[tokio::test]
    async fn a_failing_script_is_reported_and_its_head_left_for_later() {
        let mut disk = FakeDisk::default();
        disk.existing.insert(
            PathBuf::from(format!("{CLONE}/scripts/dump-schema.sh")),
            "#!".into(),
        );
        let repos = FakeRepos::default();
        let ports = fake::with(
            github(&[("main", Some("a"))], None),
            repos.clone(),
            Rc::new(disk),
            Rc::new(FakeProcess::answering(vec![FakeProcess::ran(
                "",
                "migration 0002 failed: syntax error",
                1,
            )])),
        );
        let config = config();
        let report = build(
            &ports,
            &config,
            Request {
                branches: vec!["main".into()],
            },
        )
        .run()
        .await
        .expect("ran");
        assert_eq!(
            report.snapped,
            [Snapped::Failed {
                branch: "main".into(),
                sha: "a".into(),
                reason: "scripts/dump-schema.sh exited 1: migration 0002 failed: syntax error"
                    .into(),
            }]
        );
        assert!(!repos.asked("push"), "nothing archived, nothing pushed");
    }

    #[tokio::test]
    async fn a_refused_push_fails_every_head_it_carried() {
        let mut disk = FakeDisk::default();
        disk.existing
            .insert(PathBuf::from(format!("{CLONE}/x")), String::new());
        let ports = fake::with(
            github(&[("main", Some("a")), ("main_agent", Some("b"))], None),
            FakeRepos::failing(&["push"]),
            Rc::new(disk),
            Rc::new(FakeProcess::answering(vec![
                FakeProcess::ran("A", "", 0),
                FakeProcess::ran("B", "", 0),
            ])),
        );
        let config = config();
        let report = build(
            &ports,
            &config,
            Request {
                branches: vec!["main".into(), "main_agent".into()],
            },
        )
        .run()
        .await
        .expect("ran");
        assert_eq!(report.snapped.len(), 2);
        assert!(report.snapped.iter().all(
            |s| matches!(s, Snapped::Failed { reason, .. } if reason.starts_with("git push"))
        ));
    }
}
