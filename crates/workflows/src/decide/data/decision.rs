//! What is asked, and what the session decided — read and checked here,
//! with no I/O.

use serde::Deserialize;

use crate::common::decision::Question;
use crate::common::json_reply::{self, ParseError};
use crate::dev_loop::data::missing::{self, Asked};
use crate::split::data::plan::TaskItem;

/// What a decision may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The decision is written; the task goes on from it.
    Answer,
    /// The task's PR is merged as it is.
    Merge,
    /// The task's PR is repaired once more, its repairs counted anew.
    Repair,
    /// The task is closed, with why.
    Cut,
    /// The task waits for another issue — a milestone or a task — to land,
    /// through a GitHub blocker: nothing is paid until it does.
    Wait {
        /// The issue waited for.
        on: u64,
    },
    /// The missing tasks a stop declared are approved: each opened under
    /// the milestone, and the task blocked on them until they land — the
    /// stop handler's own opener (`common::missing_task`).
    Open,
}

impl Action {
    /// How the decision names it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Answer => "answer",
            Self::Merge => "merge",
            Self::Repair => "repair",
            Self::Cut => "cut",
            Self::Wait { .. } => "wait",
            Self::Open => "task",
        }
    }
}

/// What the session answers.
#[derive(Debug, Clone, Deserialize)]
struct Answered {
    action: String,
    summary: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    confidence: u8,
    /// For a `wait`: the issue waited for, as a number or as `#20`.
    #[serde(default)]
    on: Option<serde_json::Value>,
    /// For a `task`: the slices to open, as `/split` writes them.
    #[serde(default)]
    tasks: Vec<serde_json::Value>,
}

/// An issue number as a session writes it: `20`, `"20"` or `"#20"`.
fn issue_number(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.trim().trim_start_matches('#').parse().ok(),
        _ => None,
    }
}

/// A decision, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    /// What to do.
    pub action: Action,
    /// The decision, in a sentence.
    pub summary: String,
    /// What the next session reads under `## Decision`: the answers, the
    /// choices made, markdown.
    pub answer: String,
    /// How sure it was, 1 to 5.
    pub confidence: u8,
    /// For a `task`: the slices to open, each whole.
    pub tasks: Vec<TaskItem>,
}

/// What a decision may do on the issue it is about: its number, its PR, and
/// the integration branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope<'a> {
    /// The issue decided on.
    pub number: u64,
    /// Its pull request, when it has one.
    pub pr: Option<&'a harness_core::domain::Pr>,
    /// The integration branch (`main_agent`).
    pub integration: &'a str,
    /// The rules' task: its PR into the integration branch is the one a
    /// decision may merge as it is (`review::decided_to_merge`).
    pub rules: bool,
    /// For a stop that declared missing tasks, the milestone they would
    /// open under: what lets a decision approve them (`task`).
    pub milestone: Option<u64>,
}

impl Scope<'_> {
    /// Whether the PR lands on the integration branch and is not the rules'
    /// task's: a milestone's landing PR, never merged red nor cut by a
    /// decision — a merge would put a red milestone on the integration
    /// branch, a cut would close the milestone.
    #[must_use]
    pub fn lands_on_integration(&self) -> bool {
        !self.rules && self.pr.is_some_and(|pr| pr.base == self.integration)
    }

    /// The actions this scope allows, as the prompt lists them.
    #[must_use]
    pub fn actions(&self) -> &'static str {
        if self.pr.is_none() && self.milestone.is_some() {
            "`task`, `answer`, `wait` or `cut`"
        } else if self.pr.is_none() {
            "`answer`, `wait` or `cut`"
        } else if self.lands_on_integration() {
            "`repair`, `wait` or `answer` — this PR lands a milestone on the integration \
             branch: never `merge` it red, never `cut` the milestone"
        } else {
            "`answer`, `merge`, `repair`, `wait` or `cut`"
        }
    }
}

/// The decision a session answered, within `scope`.
///
/// # Errors
///
/// No JSON object, an action it does not have, a `merge` or a `repair`
/// without a PR, a `merge` or a `cut` of a PR landing on the integration
/// branch, a `wait` naming no issue or the issue itself, an empty summary,
/// and an `answer` or a `repair` that tells a session to sync a branch with
/// the integration branch itself or push to a milestone's branch
/// (`common::branch_sync`): the harness catches branches up.
pub fn parse(text: &str, scope: &Scope<'_>) -> Result<Decided, String> {
    let number = scope.number;
    let has_pr = scope.pr.is_some();
    let answered: Answered =
        json_reply::parse_object(text).map_err(|e: ParseError| e.to_string())?;
    let action = match answered.action.trim().to_lowercase().as_str() {
        "answer" | "proceed" => Action::Answer,
        "merge" => Action::Merge,
        "repair" => Action::Repair,
        "cut" => Action::Cut,
        "wait" => {
            let on = answered.on.as_ref().and_then(issue_number).ok_or_else(|| {
                "`wait` names no issue: give `on`, the number of the issue waited for".to_string()
            })?;
            if on == number {
                return Err(format!(
                    "`wait` on #{number} itself: nothing would ever land"
                ));
            }
            Action::Wait { on }
        }
        "task" | "open" => Action::Open,
        _ => {
            return Err(format!(
                "`{}` is no action: answer, merge, repair, wait, cut or task",
                answered.action
            ));
        }
    };
    let tasks = if action == Action::Open {
        approved(&answered.tasks, scope)?
    } else {
        Vec::new()
    };
    if action == Action::Answer && scope.milestone.is_some() && approves_in_words(&answered) {
        return Err(
            "it approves a declared task in an `answer`, which opens nothing: approve it with \
             `task` and its slices in `tasks`, or answer how the task does without it"
                .to_string(),
        );
    }
    if !has_pr && matches!(action, Action::Merge | Action::Repair) {
        return Err(format!(
            "`{}` needs a pull request, and this issue has none",
            action.name()
        ));
    }
    if scope.lands_on_integration() && matches!(action, Action::Merge | Action::Cut) {
        return Err(format!(
            "`{}` of a PR that lands a milestone on `{}`: `repair` it, or `wait` on the issue \
             it needs — never merge it red, never cut the milestone",
            action.name(),
            scope.integration
        ));
    }
    if answered.summary.trim().is_empty() {
        return Err("the decision has no summary".to_string());
    }
    if matches!(action, Action::Answer | Action::Repair)
        && let Some(clause) = crate::common::branch_sync::prescribed(
            &format!("{}\n{}", answered.summary, answered.answer),
            scope.integration,
            scope.pr.map(|pr| crate::common::branch_sync::Own {
                head: &pr.head,
                base: &pr.base,
            }),
        )
    {
        return Err(format!(
            "it prescribes a branch sync: \"{clause}\" — the harness catches milestone branches \
             up and refreshes task PRs itself; answer `wait` on the issue that produces what \
             is missing instead"
        ));
    }
    Ok(Decided {
        action,
        summary: answered.summary.trim().to_string(),
        answer: answered.answer.trim().to_string(),
        confidence: answered.confidence.clamp(1, 5),
        tasks,
    })
}

/// The slices a `task` decision approves, each whole by the stop
/// handler's own measure — or why it cannot open them.
fn approved(tasks: &[serde_json::Value], scope: &Scope<'_>) -> Result<Vec<TaskItem>, String> {
    if scope.milestone.is_none() || scope.pr.is_some() {
        return Err(
            "`task` approves the missing tasks a stop declared, and this issue's stop declared \
             none under a known milestone"
                .to_string(),
        );
    }
    if tasks.is_empty() {
        return Err("`task` gives no slice: give `tasks`, each as /split writes it".to_string());
    }
    if tasks.len() > missing::MAX_SLICES {
        return Err(format!(
            "`task` gives {} slices — at most {}",
            tasks.len(),
            missing::MAX_SLICES
        ));
    }
    let mut whole = Vec::new();
    let mut gaps = Vec::new();
    for slice in tasks {
        let text = slice.to_string();
        match serde_json::from_value::<TaskItem>(slice.clone()) {
            Ok(item) => match missing::judged(item, &text) {
                Asked::Task(item) => whole.push(*item),
                Asked::Refused { title, why, .. } => gaps.push(format!("\"{title}\": {why}")),
                Asked::Unwritable { why, .. } => gaps.push(why),
            },
            Err(error) => gaps.push(format!(
                "\"{}\": a slice that does not read ({error})",
                missing::title_of(&text)
            )),
        }
    }
    if gaps.is_empty() {
        Ok(whole)
    } else {
        Err(format!(
            "`task` gives slices it cannot open: {}",
            gaps.join("; ")
        ))
    }
}

/// Whether an answer approves a declared task in words — what only `task`
/// carries out (`ForgeDesMondes`' #219: "the fixtures task is approved",
/// four times, and nothing opened).
///
/// Only an approval of a task, clause by clause: `task is approved`,
/// `task approval`, `approve the … task` — never a task and an approval of
/// something else side by side, nor a denial.
fn approves_in_words(answered: &Answered) -> bool {
    let said = format!("{}\n{}", answered.summary, answered.answer).to_lowercase();
    said.split(['.', ';', ':', ',', '!', '?', '\n'])
        .any(approves_a_task)
}

/// Whether one clause approves a task.
fn approves_a_task(clause: &str) -> bool {
    const DENIED: [&str; 6] = [
        "not",
        "never",
        "no",
        "disapprove",
        "unapproved",
        "disapproved",
    ];
    // The words a task and its approval may have between them.
    const LINKS: [&str; 9] = [
        "is", "are", "gets", "get", "got", "was", "were", "be", "now",
    ];
    // The words that end what an approval reaches.
    const BREAKS: [&str; 8] = ["and", "but", "or", "so", "then", "while", "as", "because"];
    let words: Vec<&str> = clause
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
        .filter(|word| !word.is_empty())
        .collect();
    let task = |word: &str| matches!(word, "task" | "tasks");
    if words
        .iter()
        .any(|word| DENIED.contains(word) || word.ends_with("n't") || word.contains("disapprov"))
    {
        return false;
    }
    let approval = |word: &str| matches!(word, "approved" | "approval");
    let approve = |word: &str| {
        matches!(
            word,
            "approve" | "approves" | "approved" | "approving" | "approval"
        )
    };
    words.iter().enumerate().any(|(at, word)| {
        // `task is approved`, `task approval`.
        let after_task = task(word)
            && words
                .iter()
                .skip(at + 1)
                .take(3)
                .find(|next| !LINKS.contains(next))
                .is_some_and(|next| approval(next));
        // `approve the fixtures task`.
        let before_task = approve(word)
            && words
                .iter()
                .skip(at + 1)
                .take(5)
                .take_while(|next| !BREAKS.contains(next) && !approve(next))
                .any(|next| task(next));
        after_task || before_task
    })
}

/// What kind of decision is asked.
///
/// The `kind:` of `## Decision` when it says, else what the issue shows — a
/// task waiting on its PR asks about its review, a graph question the
/// refinement's, anything else its spec.
#[must_use]
pub fn kind_of(question: Option<&Question>, review_pending: bool, rules: bool) -> String {
    if let Some(kind) = question.and_then(|q| q.kind.clone()) {
        return kind;
    }
    if rules {
        return "rules".to_string();
    }
    if review_pending {
        return "review".to_string();
    }
    if question.is_some() {
        return "graph".to_string();
    }
    "spec".to_string()
}

/// The `## Decision` section the decision leaves: the question as it was
/// asked, decided by the AI with its summary, then the answer the next
/// session reads.
#[must_use]
pub fn section(question: Option<&Question>, kind: &str, decided: &Decided) -> String {
    let mut q = question.cloned().unwrap_or_else(|| Question {
        kind: Some(kind.to_string()),
        blocks: format!("a decision was asked ({kind})"),
        round: 1,
        ..Question::default()
    });
    q.decided_by = Some("ai".to_string());
    q.verdict = Some(format!(
        "{} — {} ({}/5)",
        decided.action.name(),
        decided.summary,
        decided.confidence
    ));
    if decided.answer.is_empty() {
        q.render()
    } else {
        format!("{}\n\n{}", q.render(), decided.answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::domain::Pr;
    use proptest::prelude::*;

    fn task_pr() -> &'static Pr {
        static PR: std::sync::OnceLock<Pr> = std::sync::OnceLock::new();
        PR.get_or_init(|| Pr {
            num: "40".to_string(),
            head: "feat/pjs".to_string(),
            base: "milestone/3-x".to_string(),
            ..Pr::default()
        })
    }

    fn scope(has_pr: bool, number: u64) -> Scope<'static> {
        Scope {
            number,
            pr: has_pr.then(task_pr),
            integration: "main_agent",
            rules: false,
            milestone: None,
        }
    }

    #[test]
    fn a_decision_reads_back_and_one_without_a_pr_cannot_touch_one() {
        let text = r#"I looked. ```json
{"action": "repair", "summary": " fix the null check ", "answer": "- keep the API", "confidence": 9}
```"#;
        let read = parse(text, &scope(true, 15)).expect("read");
        assert_eq!(read.action, Action::Repair);
        assert_eq!(read.summary, "fix the null check");
        assert_eq!(read.confidence, 5, "clamped");
        assert!(
            parse(text, &scope(false, 15))
                .expect_err("no PR")
                .contains("needs a pull request")
        );
        assert!(parse(r#"{"action":"vote","summary":"x"}"#, &scope(true, 15)).is_err());
        assert!(parse(r#"{"action":"cut","summary":" "}"#, &scope(true, 15)).is_err());
        assert_eq!(
            parse(r#"{"action":"proceed","summary":"go"}"#, &scope(false, 15))
                .expect("an alias")
                .action,
            Action::Answer
        );
        assert!(parse("no json", &scope(true, 15)).is_err());
    }

    #[test]
    fn the_kind_is_the_question_s_or_what_the_issue_shows() {
        let stop = Question {
            kind: Some("stop".to_string()),
            blocks: "b".to_string(),
            ..Question::default()
        };
        assert_eq!(kind_of(Some(&stop), true, false), "stop");
        assert_eq!(kind_of(None, true, false), "review");
        assert_eq!(kind_of(None, false, true), "rules");
        let graph = Question {
            blocks: "b".to_string(),
            ..Question::default()
        };
        assert_eq!(kind_of(Some(&graph), false, false), "graph");
        assert_eq!(kind_of(None, false, false), "spec");
    }

    #[test]
    fn a_wait_names_the_issue_waited_for_and_never_the_issue_itself() {
        for on in ["20", r#""20""#, r##""#20""##, r#"" #20 ""#] {
            let text = format!(r##"{{"action":"Wait","on":{on},"summary":"#20 first"}}"##);
            assert_eq!(
                parse(&text, &scope(false, 15)).expect("read").action,
                Action::Wait { on: 20 },
                "{on}"
            );
        }
        assert_eq!(Action::Wait { on: 20 }.name(), "wait");
        assert!(
            parse(r#"{"action":"wait","summary":"x"}"#, &scope(true, 15))
                .expect_err("no issue")
                .contains("names no issue")
        );
        assert!(
            parse(
                r#"{"action":"wait","on":"soon","summary":"x"}"#,
                &scope(true, 15)
            )
            .expect_err("no number")
            .contains("names no issue")
        );
        assert!(
            parse(
                r#"{"action":"wait","on":15,"summary":"x"}"#,
                &scope(true, 15)
            )
            .expect_err("itself")
            .contains("itself")
        );
        assert_eq!(
            parse(
                r#"{"action":"answer","on":15,"summary":"x"}"#,
                &scope(true, 15)
            )
            .expect("read")
            .action,
            Action::Answer,
            "`on` means nothing to another action"
        );
    }

    #[test]
    fn the_section_keeps_the_question_and_says_the_ai_decided() {
        let decided = Decided {
            action: Action::Answer,
            summary: "Stripe".to_string(),
            answer: "- test mode first".to_string(),
            confidence: 4,
            tasks: Vec::new(),
        };
        let asked = Question {
            kind: Some("stop".to_string()),
            blocks: "which provider".to_string(),
            round: 2,
            ..Question::default()
        };
        let text = section(Some(&asked), "stop", &decided);
        let read = Question::parse(&text).expect("still a question");
        assert_eq!(read.round, 2);
        assert_eq!(read.decided_by.as_deref(), Some("ai"));
        assert_eq!(read.verdict.as_deref(), Some("answer — Stripe (4/5)"));
        assert!(text.ends_with("\n\n- test mode first"));
        let fresh = section(None, "spec", &decided);
        assert!(fresh.starts_with("kind: spec\nblocks: a decision was asked (spec)"));
    }

    proptest! {
        #[test]
        fn reading_any_answer_never_panics(text in ".{0,300}", has_pr in any::<bool>(), number in any::<u64>()) {
            let _ = parse(&text, &scope(has_pr, number));
        }
    }

    #[test]
    fn a_landing_decision_may_only_repair_wait_or_answer() {
        let landing = Pr {
            num: "423".to_string(),
            head: "milestone/22-fiche".to_string(),
            base: "main_agent".to_string(),
            ..Pr::default()
        };
        let on = Scope {
            number: 22,
            pr: Some(&landing),
            integration: "main_agent",
            rules: false,
            milestone: None,
        };
        assert!(on.lands_on_integration());
        assert!(on.actions().contains("never `merge`"));
        for action in ["merge", "cut"] {
            let text = format!(r#"{{"action":"{action}","summary":"as it is"}}"#);
            assert!(
                parse(&text, &on)
                    .expect_err(action)
                    .contains("never merge it red"),
                "{action}"
            );
        }
        assert_eq!(
            parse(r#"{"action":"repair","summary":"fix R7"}"#, &on)
                .expect("a repair")
                .action,
            Action::Repair
        );
        // The rules' task's PR into the integration branch may be merged.
        let rules = Scope { rules: true, ..on };
        assert!(!rules.lands_on_integration());
        assert!(parse(r#"{"action":"merge","summary":"faithful"}"#, &rules).is_ok());
    }

    #[test]
    fn an_answer_prescribing_a_catch_up_does_not_parse() {
        let told = r#"{"action":"repair","summary":"bring the base","answer":"Merge origin/main_agent into the branch, then fix R103."}"#;
        assert!(
            parse(told, &scope(true, 486))
                .expect_err("a sync")
                .contains("prescribes a branch sync")
        );
        let waits = r#"{"action":"answer","summary":"wait for the harness","answer":"Do not merge main_agent: the harness's catch-up brings it."}"#;
        assert!(parse(waits, &scope(true, 486)).is_ok());
        let own = r#"{"action":"repair","summary":"resolve","answer":"Merge origin/milestone/3-x into the branch and resolve Cargo.lock, then push."}"#;
        assert!(parse(own, &scope(true, 486)).is_ok(), "its own base");
    }

    /// The slice #219's decision approved in words, as `task` now carries it.
    const FIXTURES: &str = r#"{"title": "Server fixtures: load the scenes", "branch": "feat/server-scene-fixtures", "brief": "Load the scenes from mock/fixtures.", "unit": "server-fixtures", "system": "session", "aggregate": "Session", "produces": ["server.fixtures.scenes"], "goal": "The scenes are on the real stack.", "acceptance": ["given the fixtures, when the server starts, then the scenes answer"]}"#;

    fn declared_stop(has_pr: bool) -> Scope<'static> {
        Scope {
            milestone: Some(28),
            ..scope(has_pr, 219)
        }
    }

    #[test]
    fn a_task_decision_carries_whole_slices_and_only_for_a_declared_stop() {
        let text = format!(
            r#"{{"action": "task", "summary": "open the fixtures task", "tasks": [{FIXTURES}], "confidence": 4}}"#
        );
        let read = parse(&text, &declared_stop(false)).expect("an approval");
        assert_eq!(read.action, Action::Open);
        assert_eq!(read.action.name(), "task");
        assert_eq!(read.tasks.len(), 1);
        assert_eq!(read.tasks[0].unit, "infrastructure", "the alias read");
        let opened = parse(
            &text.replace(r#""task""#, r#""open""#),
            &declared_stop(false),
        );
        assert_eq!(opened.expect("an alias").action, Action::Open);
        assert!(
            parse(&text, &scope(false, 219))
                .expect_err("no declaration")
                .contains("declared none under a known milestone")
        );
        assert!(
            parse(&text, &declared_stop(true))
                .expect_err("a PR")
                .contains("declared none")
        );
        let widget = text.replace("server-fixtures", "widget");
        let refused = parse(&widget, &declared_stop(false)).expect_err("no unit");
        assert!(
            refused.contains("cannot open")
                && refused.contains(r#""Server fixtures: load the scenes""#)
                && refused.contains("one of: capability"),
            "{refused}"
        );
        assert!(
            parse(
                r#"{"action": "task", "summary": "x", "tasks": []}"#,
                &declared_stop(false)
            )
            .expect_err("empty")
            .contains("gives no slice")
        );
        let nine = format!(
            r#"{{"action": "task", "summary": "x", "tasks": [{}]}}"#,
            [FIXTURES; 9].join(",")
        );
        assert!(
            parse(&nine, &declared_stop(false))
                .expect_err("too many")
                .contains("at most 8")
        );
        let eight = format!(
            r#"{{"action": "task", "summary": "x", "tasks": [{}]}}"#,
            [FIXTURES; 8].join(",")
        );
        assert_eq!(
            parse(&eight, &declared_stop(false))
                .expect("eight, the most")
                .tasks
                .len(),
            8
        );
        let unread = parse(
            r#"{"action": "task", "summary": "x", "tasks": [{"title": "half"}]}"#,
            &declared_stop(false),
        )
        .expect_err("half a slice");
        assert!(
            unread.contains(r#""half": a slice that does not read"#),
            "{unread}"
        );
        let human = text.replace(r#""unit""#, r#""needs_human": true, "unit""#);
        assert!(
            parse(&human, &declared_stop(false))
                .expect_err("a person's")
                .contains("only a person")
        );
        assert!(declared_stop(false).actions().starts_with("`task`"));
        assert!(!scope(false, 219).actions().contains("`task`"));
        assert!(!declared_stop(true).actions().contains("`task`"));
    }

    #[test]
    fn an_answer_approving_a_declared_task_in_words_does_not_parse() {
        // #219, rounds 1–4: "the fixtures task is approved", nothing opened.
        let approved = r#"{"action": "answer", "summary": "The fixtures task is approved: it becomes a write-side task of milestone #28 that blocks #219"}"#;
        assert!(
            parse(approved, &declared_stop(false))
                .expect_err("words")
                .contains("approve it with `task`")
        );
        assert!(
            parse(approved, &scope(false, 219)).is_ok(),
            "no declaration, nothing to approve"
        );
        let in_answer =
            r#"{"action": "answer", "summary": "go", "answer": "The missing TASK is APPROVED."}"#;
        assert!(parse(in_answer, &declared_stop(false)).is_err());
    }

    #[test]
    fn only_an_approval_of_a_task_is_one() {
        let words = |summary: &str| Answered {
            action: "answer".to_string(),
            summary: summary.to_string(),
            answer: String::new(),
            confidence: 3,
            on: None,
            tasks: Vec::new(),
        };
        for approves in [
            "The fixtures task is approved: it becomes a write-side task",
            "the missing task is approved",
            "both tasks are now approved",
            "task approval granted",
            "I approve the fixtures task",
            "approve this task",
            "approval of the seed data task",
            "go.\nThe declared tasks get approved",
        ] {
            assert!(approves_in_words(&words(approves)), "{approves}");
        }
        for not in [
            "build this task as the approved mock-up shows",
            "the owner approved the port; the task uses it",
            "the owner approved the port and the task uses it",
            "I disapprove opening a task",
            "the task is not approved: build it here",
            "we don't approve this task",
            "approved: build it in this one",
            "build the task here",
            "the task is never approved",
            "approved, the task stays as it is",
            "a task with no approval",
            "the task waits for the approved port",
        ] {
            assert!(!approves_in_words(&words(not)), "{not}");
        }
    }
}
