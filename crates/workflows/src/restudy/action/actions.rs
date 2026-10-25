//! What a re-study reads for free, the streak it takes before paying, and
//! what it asks of its one session.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Halt, Issue, Outcome, Verdict};
use harness_core::execution::{Action, Context, Open, SessionAction, ask_and_record};
use harness_core::ports::shell::git::Repo;
use harness_core::ports::shell::github::GitHub;
use harness_core::ports::store::spending::Spending;

use crate::common::restudy::{self, Streak};
use crate::common::{branching, labels, sections};
use crate::restudy::action::publish::with_question;
use crate::restudy::config::ports_pair;
use crate::restudy::data::reply;
use crate::restudy::data::state::StudyState;

/// Reads, for free, what the session studies from: the task's streak, its
/// milestone and that milestone's other tasks, and the checkout's commit.
pub struct ReadCase {
    /// What reads the milestone and its sub-issues.
    pub gh: Rc<dyn GitHub>,
    /// The checkout, whose commit the session must leave as it is.
    pub git: Rc<dyn Repo>,
}

#[async_trait(?Send)]
impl Action<StudyState> for ReadCase {
    async fn run(&self, ctx: &mut Context<StudyState>) -> Outcome<Verdict> {
        let issue = ctx.state.issue().clone();
        let question = restudy::question_of(&issue).ok_or_else(|| {
            Halt::Failed(format!(
                "{} has no ## Decision to re-study",
                issue.reference()
            ))
        })?;
        let number = question.milestone.ok_or_else(|| {
            Halt::Failed(format!(
                "{}'s ## Decision names no milestone",
                issue.reference()
            ))
        })?;
        let milestone = self.gh.issue(number).await?;
        ctx.state.branch = branching::milestone_branch(milestone.number, &milestone.title);
        ctx.state.siblings = self
            .gh
            .sub_issues(number)
            .await?
            .into_iter()
            .filter(|sibling| sibling.number != issue.number)
            .collect();
        if !ctx.settings.dry_run {
            ctx.state.head = Some(self.git.head_sha().await?);
        }
        ctx.traces.say(&format!(
            "{}: re-studied on {} — {} discoveries in a row, {} open task(s) beside it",
            issue.reference(),
            ctx.state.branch,
            question.discovered,
            ctx.state.siblings.iter().filter(|s| s.is_open()).count()
        ));
        ctx.state.milestone = Some(milestone);
        ctx.state.question = Some(question);
        Ok(Verdict::Continue)
    }
}

/// Takes the streak before the session is paid for: the `restudied:`
/// marker written, so a re-study that fails is not routed again every
/// tick — one marker, one re-study, as `decide` takes its label.
///
/// A restart or a failure after it uses the re-study up: the task resumes
/// as a plain task.
pub struct TakeStreak {
    /// What writes the task's body.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<StudyState> for TakeStreak {
    async fn run(&self, ctx: &mut Context<StudyState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let issue = ctx.state.issue();
        let Some(question) = ctx.state.question.clone() else {
            return Err(Halt::Failed(
                "no question read before the streak is taken".to_string(),
            ));
        };
        let taken = Streak::of(&question).restudied().onto(question);
        self.gh
            .set_body(issue.number, &with_question(&issue.body, &taken))
            .await?;
        ctx.traces.say(&format!(
            "{}: the re-study is taken (restudied: {}, {}/{})",
            issue.reference(),
            taken.discovered,
            taken.restudies,
            restudy::MAX
        ));
        Ok(Verdict::Continue)
    }
}

/// The `## Architecture` lines that say what a sibling builds and needs.
fn architecture_of(issue: &Issue) -> String {
    sections::parse(&issue.body)
        .get(sections::ARCHITECTURE)
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| {
                    ["unit:", "id:", "produces:", "needs:"]
                        .iter()
                        .any(|key| line.starts_with(key))
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default()
}

/// One open sibling as the prompt lists it.
fn open_line(issue: &Issue) -> String {
    let branch = crate::dev_loop::data::tasks::declared_branch(&issue.body)
        .unwrap_or_else(|| "-".to_string());
    let states: Vec<&str> = issue
        .labels
        .iter()
        .map(String::as_str)
        .filter(|label| label.starts_with("harness:") && *label != labels::AGENT)
        .collect();
    format!(
        "- #{} {} — branch: {branch}; {}; [{}]",
        issue.number,
        issue.title,
        architecture_of(issue),
        states.join(", ")
    )
}

/// The values the template is spliced with.
fn values(state: &StudyState) -> Vec<(&'static str, String)> {
    let issue = state.issue();
    let question = state.question.as_ref();
    let (server, app) = ports_pair(issue.number);
    let open: Vec<String> = state
        .siblings
        .iter()
        .filter(|s| s.is_open())
        .map(open_line)
        .collect();
    let closed: Vec<String> = state
        .siblings
        .iter()
        .filter(|s| !s.is_open())
        .map(|s| format!("#{} {}", s.number, s.title))
        .collect();
    let or_none = |lines: Vec<String>, by: &str| {
        if lines.is_empty() {
            "(none)".to_string()
        } else {
            lines.join(by)
        }
    };
    vec![
        ("num", issue.number.to_string()),
        ("title", issue.title.clone()),
        (
            "milestone",
            state
                .milestone
                .as_ref()
                .map_or_else(String::new, |m| m.number.to_string()),
        ),
        ("branch", state.branch.clone()),
        (
            "discovered",
            question.map_or(0, |q| q.discovered).to_string(),
        ),
        ("server_port", server.to_string()),
        ("app_port", app.to_string()),
        ("open_slices", or_none(open, "\n")),
        ("closed_slices", or_none(closed, "; ")),
        ("body", issue.body.clone()),
        ("unit_keys", crate::dev_loop::data::missing::unit_keys()),
    ]
}

/// Sends the prompt that walks every rule, and asks once more when the
/// answer does not end on its two lines — the gate after it judges the
/// second.
pub struct AskStudy {
    /// The stage name, for the journal and the registry.
    pub stage: String,
    /// The prompt template, received from the table.
    pub template: String,
    /// Where spending is recorded.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<StudyState> for AskStudy {
    async fn run(&self, open: &mut Open<'_, StudyState>) -> Outcome<Verdict> {
        let owned = values(&open.state);
        let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let prompt = splice(&self.template, &pairs);
        let task = open.state.issue().reference();
        let mut answer =
            ask_and_record(open, &prompt, &self.stage, 1, &task, self.spending.as_ref()).await?;
        if let Err(why) = reply::parse(&answer.text) {
            let retry = format!(
                "Your answer does not read as a re-study: {why}. End your reply again with the \
                 two lines, each on a line of its own — `{}` [...] (`[]` when nothing is \
                 missing) and `{}` [...] (`[]` when none) — nothing after them.",
                reply::TASKS,
                reply::CONTEST
            );
            answer =
                ask_and_record(open, &retry, &self.stage, 1, &task, self.spending.as_ref()).await?;
        }
        open.state.studied = reply::parse(&answer.text).ok();
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::decision::Question;
    use crate::common::fake_git::FakeRepos;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::execution::Settings;
    use harness_core::ports::shell::git::Repos;
    use harness_core::traces::Logbook;

    fn task() -> Issue {
        let question = Question {
            kind: Some("wait".to_string()),
            blocks: "AGENT_LOOP_STOP: tasks [...]".to_string(),
            round: 2,
            decided_by: Some("harness".to_string()),
            verdict: Some("wait — until #567, #568 land".to_string()),
            milestone: Some(28),
            discovered: 2,
            ..Question::default()
        };
        Issue {
            number: 219,
            title: "Freeze the preparation screen".to_string(),
            state: "open".to_string(),
            labels: vec![labels::AGENT.to_string()],
            body: format!(
                "## Scope\n\nbranch: feat/preparation\n\n## Decision\n\n{}\n\n**Declared tasks** \
                 — an older note.",
                question.render()
            ),
            ..Issue::default()
        }
    }

    fn ctx(dry_run: bool) -> Context<StudyState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            StudyState {
                issue: Some(task()),
                ..StudyState::default()
            },
            Logbook::null(),
        )
    }

    fn sibling(number: u64, state: &str, labels_on: &[&str], body: &str) -> Issue {
        Issue {
            number,
            title: format!("Slice {number}"),
            state: state.to_string(),
            labels: labels_on.iter().map(ToString::to_string).collect(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    fn board() -> Rc<FakeGitHub> {
        Rc::new(FakeGitHub {
            issues: vec![
                task(),
                Issue {
                    number: 28,
                    title: "Preparation de seance".to_string(),
                    state: "open".to_string(),
                    labels: vec![labels::MILESTONE.to_string()],
                    ..Issue::default()
                },
            ],
            subs: vec![(
                28,
                vec![
                    task(),
                    sibling(
                        567,
                        "open",
                        &[labels::AGENT, labels::READY],
                        "## Scope\n\nbranch: feat/dc-update-scene-notes\n\n## Architecture\n\n\
                         unit: data-capability\nid: session.updateSceneNotes\nsystem: session\n\
                         needs: session.addScene",
                    ),
                    sibling(552, "closed", &[labels::AGENT], ""),
                ],
            )],
            ..FakeGitHub::default()
        })
    }

    #[tokio::test]
    async fn the_case_is_the_streak_the_milestone_its_branch_its_siblings_and_the_head() {
        let gh = board();
        let git = FakeRepos::default().with_heads(&["f00d"]);
        let mut context = ctx(false);
        ReadCase {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            git: git.at(std::path::Path::new("/w")),
        }
        .run(&mut context)
        .await
        .expect("read");
        assert_eq!(context.state.branch, "milestone/28-preparation-de-seance");
        assert_eq!(
            context
                .state
                .siblings
                .iter()
                .map(|s| s.number)
                .collect::<Vec<_>>(),
            [567, 552]
        );
        assert_eq!(context.state.head.as_deref(), Some("f00d"));
        assert_eq!(
            context.state.question.as_ref().map(|q| q.discovered),
            Some(2)
        );
        let spliced = values(&context.state);
        let get = |k: &str| {
            spliced
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(get("num"), "219");
        assert_eq!(get("milestone"), "28");
        assert_eq!(get("discovered"), "2");
        assert_eq!(
            (get("server_port"), get("app_port")),
            ("20438".to_string(), "20439".to_string())
        );
        assert_eq!(
            get("open_slices"),
            "- #567 Slice 567 — branch: feat/dc-update-scene-notes; unit: data-capability; id: \
             session.updateSceneNotes; needs: session.addScene; [harness:ready]"
        );
        assert_eq!(get("closed_slices"), "#552 Slice 552");
        assert!(get("unit_keys").contains("infrastructure"));
        // A dry run reads no head.
        let mut dry = ctx(true);
        ReadCase {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            git: git.at(std::path::Path::new("/w")),
        }
        .run(&mut dry)
        .await
        .expect("read");
        assert_eq!(dry.state.head, None);
    }

    #[tokio::test]
    async fn a_task_with_no_question_or_no_milestone_fails_the_case() {
        let gh = board();
        let git = FakeRepos::default();
        for body in ["## Scope\n\nx", "## Decision\n\nblocks: x\ndiscovered: 2"] {
            let mut context = ctx(false);
            context.state.issue = Some(Issue {
                body: body.to_string(),
                ..task()
            });
            let failed = ReadCase {
                gh: Rc::clone(&gh) as Rc<dyn GitHub>,
                git: git.at(std::path::Path::new("/w")),
            }
            .run(&mut context)
            .await
            .expect_err("no case");
            assert!(failed.reason().contains("#219"), "{}", failed.reason());
        }
    }

    #[tokio::test]
    async fn the_streak_is_taken_before_the_session_and_keeps_the_note_under_it() {
        let gh = board();
        let mut context = ctx(false);
        context.state.question = restudy::question_of(&task());
        TakeStreak {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("taken");
        let writes = gh.writes();
        let [Wrote::Body(219, body)] = writes.as_slice() else {
            panic!("{:?}", gh.writes());
        };
        let taken = restudy::question_of(&Issue {
            body: body.clone(),
            ..Issue::default()
        })
        .expect("a question");
        assert_eq!((taken.restudied, taken.restudies), (Some(2), 1));
        assert!(!restudy::due(&taken), "never twice on one discovery");
        assert!(
            body.trim_end()
                .ends_with("**Declared tasks** — an older note."),
            "{body}"
        );
        assert!(
            body.starts_with("## Scope\n\nbranch: feat/preparation"),
            "{body}"
        );
        // A dry run writes nothing; no question read is a failure.
        let dry = board();
        let mut context = ctx(true);
        TakeStreak {
            gh: Rc::clone(&dry) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("dry");
        assert!(dry.writes().is_empty(), "{:?}", dry.writes());
        let mut context = ctx(false);
        assert!(
            TakeStreak {
                gh: Rc::clone(&dry) as Rc<dyn GitHub>,
            }
            .run(&mut context)
            .await
            .is_err()
        );
    }

    #[test]
    fn a_sibling_without_architecture_or_labels_is_still_listed() {
        let bare = sibling(9, "open", &[labels::AGENT], "");
        assert_eq!(open_line(&bare), "- #9 Slice 9 — branch: -; ; []");
        let mut state = StudyState {
            issue: Some(task()),
            ..StudyState::default()
        };
        let none = values(&state);
        assert!(
            none.iter()
                .any(|(k, v)| *k == "open_slices" && v == "(none)")
        );
        assert!(
            none.iter()
                .any(|(k, v)| *k == "closed_slices" && v == "(none)")
        );
        assert!(none.iter().any(|(k, v)| *k == "discovered" && v == "0"));
        state.siblings = vec![bare.clone(), Issue { number: 10, ..bare }];
        let two = values(&state);
        assert!(
            two.iter()
                .any(|(k, v)| *k == "open_slices" && v.lines().count() == 2)
        );
    }
}
