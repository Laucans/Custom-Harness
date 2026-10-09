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
use harness_core::domain::{Halt, Outcome, Verdict, doctor, quota};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::github::GitHub;

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
    fn purpose(&self) -> String {
        "the harness:* labels exist on the repository".to_string()
    }

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
    fn purpose(&self) -> String {
        "GitHub answers, and an open milestone is there to work".to_string()
    }

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
    fn purpose(&self) -> String {
        "every skill the pipeline names exists under .claude/skills".to_string()
    }

    async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
        for skill in &self.named {
            let path = self.skills.join(skill).join("SKILL.md");
            if !self.disk.exists(&path) {
                return Err(Halt::Halted(format!(
                    "this run names /{skill} but .claude/skills/{skill}/SKILL.md \
                     is not in the checkout. The harness lends its own skills \
                     into the clone at mount time, so this means the harness \
                     itself has no {skill}/SKILL.md — not that the target repo \
                     should carry one"
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
    fn purpose(&self) -> String {
        "the tools the workspace needs — gh, claude, the repository's own — are installed"
            .to_string()
    }

    async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
        let missing: Vec<&(String, String)> = self
            .needed
            .iter()
            .filter(|(path, _)| !self.disk.exists(&self.root.join(path)))
            .collect();
        if missing.is_empty() {
            return Ok(Verdict::Continue);
        }
        // The marker comes from `domain::doctor`, which matches on it to run
        // the install itself: the gate is right to refuse, and the repair is to
        // carry out the very command named here.
        Err(Halt::Halted(format!(
            "the workspace has no {} — {}. Install them in {}: {}",
            missing
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            doctor::MISSING_DEPENDENCIES,
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
        // The marker, by its const: the repair matches on it, so the two
        // must not drift apart.
        assert!(err.reason().contains(doctor::MISSING_DEPENDENCIES));
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

/// Refuses a dev phase when the rate-limit window has too little left.
///
/// **What it prevents is not a refused session — those are free.** On #64, 125
/// refusals cost 0,0000 $. What cost was the *interruption*: the window ran out
/// 60 turns into `code`, the session was cut mid-reasoning, and the run that
/// resumed it spent 5,25 $ working out what the first one had left behind. The
/// first 4,32 $ delivered nothing.
///
/// So the gate is about **starting**, not about spending. It costs a file read.
///
/// **It fails open on everything but a clear refusal**, and the reasoning is
/// [`quota::decide`]'s: no reading, an unreadable one, a stale one all start. A
/// gate that blocked on absence would deadlock the loop, since the only thing
/// that can produce a reading is a run.
///
/// Disabled by `--ignore-quota`, in which case it is skipped out loud: a gate
/// that silently stopped applying would be worse than no gate.
pub struct QuotaHasRoom {
    /// What reads the recorded reading.
    pub disk: Rc<dyn Disk>,
    /// Where it was recorded — [`Workspace::quota`](harness_core::domain::workspace::Workspace::quota).
    pub at: PathBuf,
    /// `--ignore-quota`: the human has decided the reserve does not apply.
    pub ignored: bool,
    /// Now, in seconds since the epoch, so staleness is judged against a clock
    /// the caller owns rather than one this gate reaches for.
    pub now: u64,
}

#[async_trait(?Send)]
impl Verification<Loop> for QuotaHasRoom {
    fn purpose(&self) -> String {
        "the subscription's rate-limit window has room left to pay for a round".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if self.ignored {
            // `Continue`, **not** `Verdict::Skip`. `Skip` means "skip what this
            // gate guards", and `Gate` returns the first verdict that is not
            // `Continue` — so a `Skip` here skipped the four preflight checks
            // after it and then the whole run, which ended at exit 0 having done
            // nothing. `--ignore-quota` says this one check does not apply; it
            // has no business speaking for the rest.
            ctx.traces
                .say("--ignore-quota — starting whatever the rate-limit window has left");
            return Ok(Verdict::Continue);
        }
        // A file that is absent, or holds something this cannot read, is no
        // reading at all: `decide` starts on `None`.
        let reading: Option<quota::Reading> = self
            .disk
            .read_to_string(&self.at)
            .and_then(|text| serde_json::from_str(&text).ok());
        match quota::decide(reading.as_ref(), self.now) {
            quota::Verdict::Start(why) => {
                ctx.traces.debug(&format!("quota: {why}"));
                Ok(Verdict::Continue)
            }
            quota::Verdict::Wait(why) => Err(Halt::Quota(why)),
        }
    }
}

#[cfg(test)]
mod quota_gate {
    //! The gate that refuses a dev phase on a nearly spent window.
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use harness_core::domain::{Halt, Outcome, Verdict, quota};
    use harness_core::execution::{Context, Settings, Verification};
    use harness_core::ports::shell::disk::Disk;
    use harness_core::traces::Logbook;

    use super::QuotaHasRoom;
    use crate::dev_loop::data::state::Loop;

    const NOW: u64 = 1_000_000;
    const AT: &str = "/state/quota.json";

    /// A disk holding one file's text, or nothing.
    struct Holds(Option<String>);

    impl Disk for Holds {
        fn read_to_string(&self, path: &Path) -> Option<String> {
            (path == Path::new(AT)).then(|| self.0.clone()).flatten()
        }
        fn exists(&self, _path: &Path) -> bool {
            unreachable!()
        }
        fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
            unreachable!()
        }
        fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
            unreachable!()
        }
        fn dir_names(&self, _path: &Path) -> Vec<String> {
            unreachable!()
        }
        fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
            unreachable!()
        }
    }

    fn gate(held: Option<&str>, ignored: bool) -> QuotaHasRoom {
        QuotaHasRoom {
            disk: Rc::new(Holds(held.map(ToString::to_string))),
            at: PathBuf::from(AT),
            ignored,
            now: NOW,
        }
    }

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

    fn written(utilization: f64, resets_at: u64) -> String {
        serde_json::to_string(&quota::Reading {
            windows: vec![quota::Window {
                name: "five_hour".to_string(),
                utilization,
                resets_at,
            }],
            at: NOW,
        })
        .expect("serialisable")
    }

    #[tokio::test]
    async fn a_nearly_spent_window_refuses_as_a_quota_not_a_failure() {
        // `resets_at` in the future: the window is still live.
        // `Halt::Quota` and not `Halt::Failed`: the breaker treats quota as
        // transparent, so this refusal must not count towards tripping it — the
        // window will reset and the same run will then be right to proceed.
        let said = gate(Some(&written(0.96, NOW + 3600)), false)
            .verify(&ctx())
            .await;
        let Err(Halt::Quota(why)) = said else {
            panic!("a quota halt, got {said:?}")
        };
        assert!(why.contains("4%"), "{why}");
        assert!(why.contains("--ignore-quota"), "{why}");
    }

    #[tokio::test]
    async fn a_window_with_room_lets_the_run_through() {
        let said = gate(Some(&written(0.40, NOW + 3600)), false)
            .verify(&ctx())
            .await;
        assert!(matches!(said, Ok(Verdict::Continue)), "{said:?}");
    }

    #[tokio::test]
    async fn no_recorded_reading_lets_the_run_through() {
        // The deadlock this avoids: only a run can write a reading.
        let said = gate(None, false).verify(&ctx()).await;
        assert!(matches!(said, Ok(Verdict::Continue)), "{said:?}");
    }

    #[tokio::test]
    async fn an_unreadable_file_is_no_reading_rather_than_a_refusal() {
        let said = gate(Some("{ this is not a reading }"), false)
            .verify(&ctx())
            .await;
        assert!(matches!(said, Ok(Verdict::Continue)), "{said:?}");
    }

    #[tokio::test]
    async fn a_window_that_has_since_reset_lets_the_run_through() {
        // Its reset time is behind us, so what it measured is gone.
        let reset = written(1.0, NOW - 1);
        let said = gate(Some(&reset), false).verify(&ctx()).await;
        assert!(matches!(said, Ok(Verdict::Continue)), "{said:?}");
    }

    #[tokio::test]
    async fn ignore_quota_lets_the_run_through_rather_than_skipping_it() {
        // The defect this locks out, and my previous test locked *in*: this
        // returned `Verdict::Skip`, `Gate` stops on the first verdict that is not
        // `Continue`, and so `--ignore-quota` skipped the four checks after it
        // and then the entire run — exit 0, nothing done, nothing said. The old
        // test asserted `Skip` because that was what the code did; it should have
        // asserted what the flag means.
        let said = gate(Some(&written(0.99, NOW + 3600)), true)
            .verify(&ctx())
            .await;
        assert!(matches!(said, Ok(Verdict::Continue)), "{said:?}");
    }

    #[tokio::test]
    async fn ignore_quota_says_so_rather_than_applying_silently() {
        // A gate that stopped applying without saying so would be worse than no
        // gate: nobody would know which of the two runs they were looking at.
        let heard = Rc::new(Heard::default());
        let context = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            Loop::default(),
            Logbook::new(
                Rc::clone(&heard) as Rc<dyn harness_core::traces::Sink>,
                harness_core::traces::Verbosity::Normal,
            ),
        );
        gate(Some(&written(0.99, NOW + 3600)), true)
            .verify(&context)
            .await
            .expect("through");
        assert!(
            heard
                .0
                .borrow()
                .iter()
                .any(|l| l.contains("--ignore-quota")),
            "{:?}",
            heard.0.borrow()
        );
    }

    /// A console that keeps what it was told.
    #[derive(Default)]
    struct Heard(std::cell::RefCell<Vec<String>>);

    impl harness_core::traces::Sink for Heard {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    #[tokio::test]
    async fn the_gate_this_check_opens_is_not_short_circuited_by_it() {
        // The defect was only visible one level up: the check looked right on
        // its own. What broke was the *gate* — so the gate is what this asserts.
        let whole: harness_core::execution::Gate<Loop> = harness_core::execution::Gate {
            name: "preflight",
            checks: vec![
                Box::new(gate(Some(&written(0.99, NOW + 3600)), true)),
                Box::new(MustBeReached),
            ],
        };
        let said = whole.verify(&ctx()).await;
        assert!(
            matches!(said, Err(Halt::Halted(ref why)) if why == "reached"),
            "the check after --ignore-quota must still run, got {said:?}"
        );
    }

    /// Proof that the checks after the quota gate are still reached.
    struct MustBeReached;

    #[async_trait::async_trait(?Send)]
    impl Verification<Loop> for MustBeReached {
        async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
            Err(Halt::Halted("reached".to_string()))
        }
    }
}
