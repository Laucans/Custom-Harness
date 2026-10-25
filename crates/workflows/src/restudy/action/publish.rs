//! What a re-study found, opened.
//!
//! The slices go through the one opener (`common::missing_task`, which
//! chains them and parks the task on each as it opens or finds it), and
//! the task's `## Decision` says what was opened and what to contest.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Event;

use crate::common::decision::Question;
use crate::common::missing_task::{self, Found};
use crate::common::restudy::Streak;
use crate::common::sections;
use crate::dev_loop::data::missing::{self, Asked};
use crate::dev_loop::data::tasks;
use crate::restudy::data::reply::Contest;
use crate::restudy::data::state::StudyState;
use crate::split::data::plan::TaskItem;

/// `body` with its `## Decision` question replaced by `question`, the words
/// under it kept.
#[must_use]
pub fn with_question(body: &str, question: &Question) -> String {
    let section = sections::parse(body)
        .get(sections::DECISION)
        .cloned()
        .unwrap_or_default();
    let under = section
        .split_once("\n\n")
        .map(|(_, under)| under.trim())
        .unwrap_or_default();
    let text = if under.is_empty() {
        question.render()
    } else {
        format!("{}\n\n{under}", question.render())
    };
    sections::replace(body, &[(sections::DECISION, &text)])
}

/// The slices to open, and those not opened with why — declared wrong, or
/// only a person's.
fn sorted(asked: Vec<Asked>) -> (Vec<TaskItem>, Vec<(String, String)>) {
    let mut whole = Vec::new();
    let mut unopened = Vec::new();
    for one in asked {
        match one {
            Asked::Task(item) => whole.push(*item),
            Asked::Refused { title, why, .. } => unopened.push((title, why)),
            Asked::Unwritable { said, why } => unopened.push((missing::title_of(&said), why)),
        }
    }
    (whole, unopened)
}

/// The issues as a sentence names them: `#1, #2`.
fn named(numbers: &[u64]) -> String {
    numbers
        .iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the next code session reads under the question: what the re-study
/// opened and found, the rule tests to contest, the slices not opened.
fn note(
    discovered: u32,
    found: &Found,
    contests: &[Contest],
    unopened: &[(String, String)],
) -> String {
    let mut parts = Vec::new();
    let found_again: Vec<u64> = found
        .on
        .iter()
        .copied()
        .filter(|n| !found.opened.contains(n))
        .collect();
    let mut done = Vec::new();
    if !found.opened.is_empty() {
        done.push(format!("opened {}", named(&found.opened)));
    }
    if !found_again.is_empty() {
        done.push(format!("found {}", named(&found_again)));
    }
    if !found.chained.is_empty() {
        let chains: Vec<String> = found
            .chained
            .iter()
            .map(|(blocked, blocker, id)| format!("#{blocked} on #{blocker} (`{id}`)"))
            .collect();
        done.push(format!("chained {}", chains.join(", ")));
    }
    if !done.is_empty() || !contests.is_empty() || !unopened.is_empty() {
        parts.push(format!(
            "**Re-study** ({discovered} discoveries in a row): {}.",
            if done.is_empty() {
                "nothing missing beyond the open tasks".to_string()
            } else {
                done.join("; ")
            }
        ));
    }
    if !contests.is_empty() {
        let lines: Vec<String> = contests
            .iter()
            .map(|contest| {
                let at = if contest.test.trim().is_empty() {
                    String::new()
                } else {
                    format!(" (`{}`)", contest.test.trim())
                };
                format!("- {}{at} — {}", contest.rule.trim(), contest.why.trim())
            })
            .collect();
        parts.push(format!(
            "**Contest these rule tests with CONTEST.md before you build:**\n\n{}",
            lines.join("\n")
        ));
    }
    let refused = missing_task::refusal_note(unopened);
    if !refused.is_empty() {
        parts.push(refused);
    }
    parts.join("\n\n")
}

/// Opens what the re-study found, parks the task on it, writes it under
/// `## Decision`, and records `Restudied`.
pub struct Publish {
    /// What opens the slices and writes the task.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<StudyState> for Publish {
    async fn run(&self, ctx: &mut Context<StudyState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let (Some(studied), Some(question), Some(milestone)) = (
            ctx.state.studied.clone(),
            ctx.state.question.clone(),
            ctx.state.milestone.as_ref().map(|m| m.number),
        ) else {
            return Err(Halt::Failed(
                "nothing read to publish — the case or the study did not run".to_string(),
            ));
        };
        let number = ctx.state.issue().number;
        // Re-read: the body may have moved during the session; only its
        // `## Decision` is rewritten.
        let issue = self.gh.issue(number).await?;
        let (whole, mut unopened) = sorted(studied.asked);
        let found =
            missing_task::open_or_find(self.gh.as_ref(), number, milestone, &whole, &ctx.traces)
                .await?;
        unopened.extend(found.built.iter().cloned());
        let streak = Streak::of(&question).restudied();
        let note = note(question.discovered, &found, &studied.contests, &unopened);
        if !found.on.is_empty() || !note.is_empty() {
            let rewritten = if found.on.is_empty() {
                streak.onto(question.clone())
            } else {
                let mut waits: Vec<u64> = self
                    .gh
                    .blocked_by(number)
                    .await?
                    .iter()
                    .filter(|blocker| tasks::still_blocks(blocker))
                    .map(|blocker| blocker.number)
                    .collect();
                for on in &found.on {
                    if !waits.contains(on) {
                        waits.push(*on);
                    }
                }
                streak.onto(Question {
                    kind: Some("wait".to_string()),
                    blocks: question.blocks.clone(),
                    round: question.round,
                    decided_by: Some("harness".to_string()),
                    verdict: Some(format!(
                        "wait — until {} land{}",
                        named(&waits),
                        if waits.len() == 1 { "s" } else { "" }
                    )),
                    milestone: question.milestone,
                    ..Question::default()
                })
            };
            let text = if note.is_empty() {
                rewritten.render()
            } else {
                format!("{}\n\n{note}", rewritten.render())
            };
            self.gh
                .set_body(
                    number,
                    &sections::replace(&issue.body, &[(sections::DECISION, &text)]),
                )
                .await?;
        }
        for (title, why) in &unopened {
            ctx.traces.warn(&format!(
                "#{number}: the re-study's slice \"{title}\" is not opened — {why}"
            ));
        }
        let contests: Vec<String> = studied
            .contests
            .iter()
            .map(|contest| contest.rule.trim().to_string())
            .collect();
        ctx.state.published = Some(if found.on.is_empty() {
            "nothing missing beyond the open tasks".to_string()
        } else {
            format!("waits on {}", named(&found.on))
        });
        ctx.traces.event(&Event::Restudied {
            task: number,
            on: found.on,
            contests,
        });
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::common::{labels, restudy};
    use crate::restudy::data::reply;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::{Logbook, Sink, Verbosity};

    const ROUND_6: &str = include_str!("../../../tests/fixtures/stop-219-round-6.txt");

    /// A sink that keeps the lines and the events told.
    #[derive(Default)]
    struct Told(RefCell<(Vec<String>, Vec<Event>)>);

    impl Sink for Told {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().0.push(line.to_string());
        }
        fn record(&self, event: &Event) {
            self.0.borrow_mut().1.push(event.clone());
        }
    }

    /// #219 as the re-study took it: due at 2, its streak taken.
    fn task() -> Issue {
        let question = Question {
            kind: Some("wait".to_string()),
            blocks: "AGENT_LOOP_STOP: tasks [...]".to_string(),
            round: 2,
            decided_by: Some("harness".to_string()),
            verdict: Some("wait — until #563 lands".to_string()),
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
                "## Scope\n\nbranch: feat/preparation\n\n## Decision\n\n{}",
                Streak::of(&question).restudied().onto(question).render()
            ),
            ..Issue::default()
        }
    }

    fn question() -> Question {
        Question {
            restudied: None,
            restudies: 0,
            ..restudy::question_of(&task()).expect("a question")
        }
    }

    fn ctx(text: &str, dry_run: bool) -> (Rc<Told>, Context<StudyState>) {
        let sink = Rc::new(Told::default());
        let ctx = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            StudyState {
                issue: Some(task()),
                question: Some(question()),
                milestone: Some(Issue {
                    number: 28,
                    ..Issue::default()
                }),
                studied: reply::parse(text).ok(),
                ..StudyState::default()
            },
            Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal),
        );
        (sink, ctx)
    }

    fn board(subs: Vec<Issue>) -> Rc<FakeGitHub> {
        let gh = Rc::new(FakeGitHub {
            issues: vec![task()],
            subs: vec![(28, subs)],
            ..FakeGitHub::default()
        });
        gh.next_issue_number.set(566);
        gh
    }

    fn body_of(writes: &[Wrote]) -> String {
        writes
            .iter()
            .rev()
            .find_map(|w| match w {
                Wrote::Body(219, body) => Some(body.clone()),
                _ => None,
            })
            .expect("the task's body rewritten")
    }

    fn found_reply() -> String {
        format!(
            "Walked R11, R13, R15.\n{} {}\n{} [{{\"rule\": \"R15\", \"test\": \
             \"tests/rules/journey/R15.spec.ts:42\", \"why\": \"the heading's position depends \
             on the scroll\"}}]",
            reply::TASKS,
            ROUND_6.trim().trim_start_matches("AGENT_LOOP_STOP: tasks "),
            reply::CONTEST
        )
    }

    #[tokio::test]
    async fn the_re_study_opens_chains_parks_and_says_what_to_contest() {
        let blocker = Issue {
            number: 563,
            state: "open".to_string(),
            labels: vec![labels::AGENT.to_string()],
            ..Issue::default()
        };
        let mut waiting = task();
        waiting.blocked_by = vec![blocker.clone()];
        let gh = board(vec![waiting, blocker]);
        let (sink, mut context) = ctx(&found_reply(), false);
        Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("published");
        let writes = gh.writes();
        assert!(
            writes.contains(&Wrote::BlockedByLink(568, 567)),
            "{writes:?}"
        );
        assert!(
            writes.contains(&Wrote::BlockedByLink(219, 567)),
            "{writes:?}"
        );
        assert!(
            writes.contains(&Wrote::BlockedByLink(219, 568)),
            "{writes:?}"
        );
        let body = body_of(&writes);
        let written = restudy::question_of(&Issue {
            body: body.clone(),
            ..Issue::default()
        })
        .expect("a question");
        assert_eq!(written.kind.as_deref(), Some("wait"));
        assert_eq!(written.round, 2, "the round the stops counted");
        assert_eq!(written.decided_by.as_deref(), Some("harness"));
        assert_eq!(
            written.verdict.as_deref(),
            Some("wait — until #563, #567, #568 land")
        );
        assert_eq!(
            (written.discovered, written.restudied, written.restudies),
            (2, Some(2), 1)
        );
        assert_eq!(written.milestone, Some(28));
        assert!(!restudy::due(&written));
        assert!(
            body.contains(
                "**Re-study** (2 discoveries in a row): opened #567, #568; chained #568 on #567 \
                 (`session.updateSceneNotes`)."
            ),
            "{body}"
        );
        assert!(
            body.contains(
                "**Contest these rule tests with CONTEST.md before you build:**\n\n- R15 \
                 (`tests/rules/journey/R15.spec.ts:42`) — the heading's position depends on the \
                 scroll"
            ),
            "{body}"
        );
        assert!(body.starts_with("## Scope\n\nbranch: feat/preparation"));
        let events = sink.0.borrow().1.clone();
        assert!(
            events.contains(&Event::Restudied {
                task: 219,
                on: vec![567, 568],
                contests: vec!["R15".to_string()],
            }),
            "{events:?}"
        );
        assert_eq!(
            context.state.published.as_deref(),
            Some("waits on #567, #568")
        );
    }

    #[tokio::test]
    async fn an_open_task_declared_again_with_a_new_need_is_chained_on_the_new_slice() {
        // #568 open already (the Scene board), declared again by its branch
        // with the notes capability it needs: found, not reopened, chained.
        let scene_board = Issue {
            number: 568,
            title: "Scene board writes".to_string(),
            state: "open".to_string(),
            labels: vec![
                labels::AGENT.to_string(),
                labels::REVIEW_PENDING.to_string(),
            ],
            body: "## Scope\n\nbranch: feat/micro-ui-scene-board-writes-on-real-store\n\n\
                   Opened by the harness for #219, whose session stopped on it."
                .to_string(),
            ..Issue::default()
        };
        let gh = board(vec![task(), scene_board]);
        let (_, mut context) = ctx(&found_reply(), false);
        Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("published");
        let writes = gh.writes();
        let created: Vec<&str> = writes
            .iter()
            .filter_map(|w| match w {
                Wrote::CreatedIssue(title, ..) => Some(title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(created.len(), 1, "{created:?}");
        assert!(created[0].starts_with("DataCapability"), "{created:?}");
        assert!(
            writes.contains(&Wrote::BlockedByLink(568, 567)),
            "{writes:?}"
        );
        assert!(
            writes.contains(&Wrote::BlockedByLink(219, 567)),
            "{writes:?}"
        );
        let body = body_of(&writes);
        assert!(
            body.contains("opened #567; found #568; chained #568 on #567"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn nothing_missing_writes_only_the_marker_already_taken() {
        let gh = board(vec![task()]);
        let (sink, mut context) = ctx("All built.\nRESTUDY: tasks []", false);
        Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("published");
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
        assert_eq!(
            context.state.published.as_deref(),
            Some("nothing missing beyond the open tasks")
        );
        assert_eq!(
            sink.0.borrow().1,
            [Event::Restudied {
                task: 219,
                on: Vec::new(),
                contests: Vec::new(),
            }]
        );
        // Only a contest: written, the question kept as it was.
        let gh = board(vec![task()]);
        let (_, mut context) = ctx(
            "RESTUDY: tasks []\nRESTUDY_CONTEST: [{\"rule\": \"R15\", \"why\": \"scroll\"}]",
            false,
        );
        Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("published");
        let body = body_of(&gh.writes());
        let kept = restudy::question_of(&Issue {
            body: body.clone(),
            ..Issue::default()
        })
        .expect("q");
        assert_eq!(kept.verdict.as_deref(), Some("wait — until #563 lands"));
        assert_eq!((kept.restudied, kept.restudies), (Some(2), 1));
        assert!(
            body.contains("nothing missing beyond the open tasks.\n\n**Contest these rule tests"),
            "{body}"
        );
        assert!(body.contains("- R15 — scroll"), "{body}");
    }

    #[tokio::test]
    async fn a_slice_declared_wrong_or_built_already_goes_into_the_note() {
        let built = Issue {
            number: 500,
            state: "closed".to_string(),
            labels: vec![labels::AGENT.to_string()],
            body: "## Scope\n\nbranch: feat/dc-update-scene-notes\n".to_string(),
            ..Issue::default()
        };
        let gh = board(vec![task(), built]);
        let wrong = found_reply().replacen("\"unit\":\"micro-ui\"", "\"unit\":\"widget\"", 1);
        let (sink, mut context) = ctx(&wrong, false);
        Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut context)
        .await
        .expect("published");
        let writes = gh.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::CreatedIssue(..))),
            "{writes:?}"
        );
        let body = body_of(&writes);
        assert!(
            body.contains("**Declared tasks the harness did not open**"),
            "{body}"
        );
        assert!(
            body.contains("#500 built `feat/dc-update-scene-notes` already"),
            "{body}"
        );
        assert!(body.contains("is no unit of the architecture"), "{body}");
        let lines = sink.0.borrow().0.clone();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#219: the re-study's slice \"Micro-UI SceneBoard")),
            "{lines:?}"
        );
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing_and_no_study_read_fails() {
        let gh = board(vec![task()]);
        let (_, mut dry) = ctx(&found_reply(), true);
        Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        }
        .run(&mut dry)
        .await
        .expect("dry");
        assert!(gh.writes().is_empty(), "{:?}", gh.writes());
        let (_, mut unread) = ctx("prose", false);
        assert!(
            Publish {
                gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            }
            .run(&mut unread)
            .await
            .is_err()
        );
    }

    #[test]
    fn the_question_is_replaced_and_the_words_under_it_kept() {
        let q = Question {
            blocks: "x".to_string(),
            round: 1,
            ..Question::default()
        };
        assert_eq!(
            with_question("## Decision\n\nblocks: old\nround: 3\n\nunder it", &q),
            "## Decision\n\nblocks: x\nround: 1\n\nunder it\n"
        );
        assert_eq!(
            with_question("## Scope\n\ns\n\n## Decision\n\nblocks: old", &q),
            "## Scope\n\ns\n\n## Decision\n\nblocks: x\nround: 1\n"
        );
        assert_eq!(
            sorted(vec![Asked::Unwritable {
                said: r#"{"title": "Pay"}"#.to_string(),
                why: "a person's".to_string(),
            }])
            .1,
            [("Pay".to_string(), "a person's".to_string())]
        );
        assert_eq!(note(2, &Found::default(), &[], &[]), "");
    }
}
