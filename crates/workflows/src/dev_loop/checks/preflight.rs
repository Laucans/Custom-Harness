//! What is verified before the first stage is paid.
//!
//! Each gate here cost a run to write. A typo in a label costs nothing to find
//! here, and a whole `/code` to find once `claude` is billed.
//!
//! **A gate decides, it never calls `gh`/`git`/`which` itself**: each holds
//! the port it needs, and the adapter talks to the system. Tooling gates —
//! `claude` on PATH at the right version, `gh` authenticated — live in the
//! launcher: they belong to no workflow, and the launcher wires them.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::disk::Disk;
use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::common::labels;
use crate::dev_loop::data::board;
use crate::dev_loop::data::state::Loop;

/// The seven labels everything the model is expressed in.
///
/// A non-existent label fails **nowhere**: querying it returns an empty list,
/// and an empty task list reads "this milestone is done" — the only thing
/// that pays for `/planner`. A typo is worth catching here, free.
pub struct LabelsExist {
    /// What lists the repo's labels.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Verification<Loop> for LabelsExist {
    async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
        let known = self.gh.labels().await?;
        let missing: Vec<&str> = labels::LOOP
            .into_iter()
            .filter(|wanted| !known.iter().any(|here| here == wanted))
            .collect();
        if missing.is_empty() {
            return Ok(Verdict::Continue);
        }
        let fix = missing
            .iter()
            .map(|label| format!("gh label create {label}"))
            .collect::<Vec<_>>()
            .join("; ");
        Err(Halt::Halted(format!(
            "the repository is missing the label(s) {} — create them with: {fix}",
            missing.join(" ")
        )))
    }
}

/// There is an open milestone, and GitHub answered when asked.
///
/// `board::read` fails on both counts, with prose naming the gesture; running
/// it here is what keeps these two failures from landing mid-round after a
/// stage is billed.
pub struct MilestoneIsReachable {
    /// What reads the board.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Verification<Loop> for MilestoneIsReachable {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        let here = board::read(self.gh.as_ref()).await?;
        ctx.traces.debug(&format!(
            "milestone {}: {} — {} open task(s)",
            here.milestone.reference(),
            here.milestone.title,
            here.open_agents().len()
        ));
        Ok(Verdict::Continue)
    }
}

/// The table names skills: their files must exist.
///
/// `lead` counts too — `/tech-analyst` is not the name of any table entry,
/// so nothing else would see it missing.
pub struct SkillsExist {
    /// What asks if a file is there.
    pub disk: Rc<dyn Disk>,
    /// `.claude/skills` of the workspace.
    pub skills: PathBuf,
    /// The skills this run names — stages and `lead` both.
    pub named: Vec<String>,
}

#[async_trait(?Send)]
impl Verification<Loop> for SkillsExist {
    async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
        for skill in &self.named {
            let path = self.skills.join(skill).join("SKILL.md");
            if !self.disk.exists(&path) {
                return Err(Halt::Halted(format!(
                    "this run names /{skill} but .claude/skills/{skill}/SKILL.md \
                     does not exist"
                )));
            }
        }
        Ok(Verdict::Continue)
    }
}

/// What a stage must find in the workspace for its verification commands to
/// even start.
///
/// **The gate that makes workspace passage bearable.** A clone carries only
/// what git tracks: no npm dependencies, no venv. A fresh workspace thus
/// passes all other gates — it's clean by construction — then burns a paid
/// session whose every verification command answers "command not found".
///
/// A gate costs a `stat`. The round it avoids costs a `/code`.
///
/// **Last** in the list: it's the only one that talks about what's *in* the
/// workspace rather than what it is, and it makes sense only once you know
/// the workspace is the one you think.
pub struct DependenciesAreInstalled {
    /// What asks if a path is there.
    pub disk: Rc<dyn Disk>,
    /// The root of the code.
    pub root: PathBuf,
    /// What must be there, and how to restore it.
    pub needed: Vec<(String, String)>,
}

#[async_trait(?Send)]
impl Verification<Loop> for DependenciesAreInstalled {
    async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
        let missing: Vec<&(String, String)> = self
            .needed
            .iter()
            .filter(|(path, _)| !self.disk.exists(&self.root.join(path)))
            .collect();
        if missing.is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "the workspace has no {} — nothing a clone carries. Install them in \
             {}: {}",
            missing
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            self.root.display(),
            missing
                .iter()
                .map(|(_, fix)| fix.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use std::collections::HashSet;
    use std::path::Path;

    fn ctx() -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            Loop::default(),
            Logbook::null(),
        )
    }

    /// A disk where only these paths exist.
    struct Only(HashSet<PathBuf>);

    impl Only {
        fn with(paths: &[&str]) -> Rc<Self> {
            Rc::new(Self(paths.iter().map(PathBuf::from).collect()))
        }
    }

    impl Disk for Only {
        fn exists(&self, path: &Path) -> bool {
            self.0.contains(path)
        }

        fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
            Ok(())
        }

        fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
            Ok(())
        }

        fn dir_names(&self, _path: &Path) -> Vec<String> {
            Vec::new()
        }

        fn read_to_string(&self, _path: &Path) -> Option<String> {
            None
        }

        fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn every_label_the_model_needs_is_demanded_and_the_fix_is_named() {
        let gh = Rc::new(FakeGitHub {
            labels: vec![labels::MILESTONE.to_string(), labels::AGENT.to_string()],
            ..FakeGitHub::default()
        });
        let err = LabelsExist { gh }
            .verify(&ctx())
            .await
            .expect_err("must stop");
        assert!(err.reason().contains(labels::READY));
        assert!(
            err.reason().contains("gh label create"),
            "a gate names the gesture"
        );
        // What's already there is not re-demanded.
        assert!(!err.reason().contains(&format!("create {}", labels::AGENT)));
    }

    #[tokio::test]
    async fn all_seven_labels_present_lets_the_run_start() {
        let gh = Rc::new(FakeGitHub {
            labels: labels::LOOP.iter().map(ToString::to_string).collect(),
            ..FakeGitHub::default()
        });
        assert_eq!(
            LabelsExist { gh }.verify(&ctx()).await.expect("a verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn an_unreadable_label_list_is_not_an_absent_label() {
        // The difference is worth a `/planner`: "I don't know" is not "absent".
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("expired token".to_string())),
            ..FakeGitHub::default()
        });
        let err = LabelsExist { gh }
            .verify(&ctx())
            .await
            .expect_err("must fail");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[tokio::test]
    async fn a_reachable_milestone_passes_without_paying_anything() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![Issue {
                number: 4,
                state: "open".to_string(),
                labels: vec![labels::MILESTONE.to_string()],
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        assert_eq!(
            MilestoneIsReachable { gh }
                .verify(&ctx())
                .await
                .expect("a verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_skill_the_table_names_but_the_repo_lacks_stops_the_run() {
        let gate = SkillsExist {
            disk: Only::with(&["/w/.claude/skills/code/SKILL.md"]),
            skills: PathBuf::from("/w/.claude/skills"),
            named: vec!["code".to_string(), "tech-analyst".to_string()],
        };
        let err = gate.verify(&ctx()).await.expect_err("must stop");
        // `/tech-analyst` is a `lead`: the name of no table entry, so nothing
        // else would see it missing.
        assert!(err.reason().contains("tech-analyst"));
    }

    #[tokio::test]
    async fn a_fresh_clone_without_dependencies_stops_before_burning_a_session() {
        // A fresh workspace passes all other gates — it's clean by construction
        // — then burns a /code whose every command answers "command not found".
        let gate = DependenciesAreInstalled {
            disk: Only::with(&[]),
            root: PathBuf::from("/w"),
            needed: vec![("node_modules".to_string(), "npm install".to_string())],
        };
        let err = gate.verify(&ctx()).await.expect_err("must stop");
        assert!(err.reason().contains("node_modules"));
        assert!(err.reason().contains("npm install"));
        assert!(err.reason().contains("nothing a clone carries"));
    }

    #[tokio::test]
    async fn an_installed_workspace_passes() {
        let gate = DependenciesAreInstalled {
            disk: Only::with(&["/w/node_modules"]),
            root: PathBuf::from("/w"),
            needed: vec![("node_modules".to_string(), "npm install".to_string())],
        };
        assert_eq!(
            gate.verify(&ctx()).await.expect("a verdict"),
            Verdict::Continue
        );
    }
}
