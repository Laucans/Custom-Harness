//! Is every acceptance criterion of the task proved by a test once `code`
//! is done? A gate on the round, local and deterministic.
//!
//! The prompt numbers the criteria `AC1..ACn` ([`acceptance::numbered`])
//! and asks for a comment naming each in the test that proves it
//! (`// AC2: …`). This gate reads the task's pull request: every criterion
//! must be named by a test file the PR changes — more often at its head
//! than on its base, so a name an earlier task left proves nothing — or
//! pointed to an existing test in its body
//! (`AC4: covered by tests/rules/R103.spec.ts — …`, the file a test at the
//! PR's head), or declared there with a reason
//! (`AC3: not testable here — …`). One missing fails the round: the PR gets
//! a blocking review naming the criteria, and its repair is asked — as
//! [`ArchitectureHolds`](crate::dev_loop::checks::architecture::ArchitectureHolds)
//! does for a drift.
//!
//! A task without criteria, without a PR yet, or whose PR is no longer
//! open is not judged here: the delivery check speaks for the first two, and
//! a merged PR is past refusing. Nor is a PR GitHub could not read whole —
//! its files, its body, one file's text: a failed read is no proof of a
//! missing test, and a blocking review on it would pay a repair for
//! nothing.

use std::collections::BTreeSet;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Pr, Verdict};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::github::GitHub;

use crate::dev_loop::checks::architecture::{Refusal, ask_repair};
use crate::dev_loop::data::acceptance::{self, Changed};
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::tasks;

/// How many of the PR's source files are read, at most: a PR past it is
/// judged on its first ones.
pub const FILES_READ: usize = 120;

/// Every criterion is named by a test, pointed to one or declared
/// untestable: see the module.
pub struct CriteriaTested {
    /// What reads the task's pull request.
    pub gh: Rc<dyn GitHub>,
}

/// Says why the PR is not judged — a read GitHub failed — and lets the
/// round through.
fn unread(ctx: &Context<Loop>, pr: &Pr, what: &str, halt: &Halt) {
    ctx.traces.warn(&format!(
        "criteria: {what} of PR {} cannot be read — not judged ({})",
        pr.reference(),
        halt.reason()
    ));
}

impl CriteriaTested {
    /// The PR's source files, each with its text on the base and at the
    /// head — a file the PR deletes is left out, one it adds has an empty
    /// base. `None` when GitHub fails a read.
    async fn sources(&self, ctx: &Context<Loop>, pr: &Pr) -> Option<Vec<Changed>> {
        let files = match self.gh.pr_files(&pr.num).await {
            Ok(files) => files,
            Err(halt) => {
                unread(ctx, pr, "the files", &halt);
                return None;
            }
        };
        let mut read = Vec::new();
        for path in files
            .into_iter()
            .filter(|path| acceptance::may_hold_tests(path))
            .take(FILES_READ)
        {
            let Some(head) = self.text(ctx, pr, &path, &pr.head).await? else {
                continue;
            };
            let base = self.text(ctx, pr, &path, &pr.base).await?;
            read.push(Changed {
                path,
                base: base.unwrap_or_default(),
                head,
            });
        }
        Some(read)
    }

    /// One file's text at `git_ref`: `Some(None)` when it is not there,
    /// `None` when GitHub fails the read.
    async fn text(
        &self,
        ctx: &Context<Loop>,
        pr: &Pr,
        path: &str,
        git_ref: &str,
    ) -> Option<Option<String>> {
        match self.gh.file_text(path, git_ref).await {
            Ok(text) => Some(text),
            Err(halt) => {
                unread(ctx, pr, &format!("`{path}` at {git_ref}"), &halt);
                None
            }
        }
    }

    /// The PR's body, `None` when GitHub fails the read.
    async fn body(&self, ctx: &Context<Loop>, pr: &Pr) -> Option<String> {
        let read = match pr.num.parse::<u64>() {
            Ok(number) => self.gh.issue(number).await,
            Err(_) => Err(Halt::Unreadable(format!("PR number `{}`", pr.num))),
        };
        match read {
            Ok(issue) => Some(issue.body),
            Err(halt) => {
                unread(ctx, pr, "the body", &halt);
                None
            }
        }
    }

    /// The criteria the PR body points to an existing test that proves
    /// them at the PR's head; `None` when GitHub fails a read.
    async fn proven(&self, ctx: &Context<Loop>, pr: &Pr, body: &str) -> Option<BTreeSet<usize>> {
        let mut proven = BTreeSet::new();
        for pointer in acceptance::declared_covered(body).iter().take(FILES_READ) {
            let text = self.text(ctx, pr, &pointer.path, &pr.head).await?;
            if text.is_some_and(|text| acceptance::proves(pointer, &text)) {
                proven.extend(&pointer.criteria);
            }
        }
        Some(proven)
    }
}

/// What the review and the halt list, one line per criterion left bare.
#[must_use]
pub fn findings(criteria: &[String], uncovered: &[usize]) -> Vec<String> {
    uncovered
        .iter()
        .map(|number| {
            let text = number
                .checked_sub(1)
                .and_then(|at| criteria.get(at))
                .map_or("", String::as_str);
            format!(
                "AC{number} ({text}): named by no test this PR adds, pointed to no existing test, \
                 declared nowhere"
            )
        })
        .collect()
}

#[async_trait(?Send)]
impl Verification<Loop> for CriteriaTested {
    fn purpose(&self) -> String {
        "every acceptance criterion AC1..ACn of the task is named by a test the pull request adds or changes, pointed to an existing test, or declared untestable with its reason in the PR body".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let criteria = acceptance::of_body(&ctx.state.task.body);
        if criteria.is_empty() {
            return Ok(Verdict::Continue);
        }
        let Some(branch) = tasks::declared_branch(&ctx.state.task.body) else {
            return Ok(Verdict::Continue);
        };
        let Ok(pr) = self.gh.pr(&branch).await else {
            return Ok(Verdict::Continue);
        };
        if !pr.is_open() {
            ctx.traces.say(&format!(
                "criteria: PR {} is {} — past judging",
                pr.reference(),
                pr.state.to_lowercase()
            ));
            return Ok(Verdict::Continue);
        }
        let Some(sources) = self.sources(ctx, &pr).await else {
            return Ok(Verdict::Continue);
        };
        let Some(body) = self.body(ctx, &pr).await else {
            return Ok(Verdict::Continue);
        };
        let Some(proven) = self.proven(ctx, &pr, &body).await else {
            return Ok(Verdict::Continue);
        };
        let uncovered = acceptance::uncovered(criteria.len(), &sources, &body, &proven);
        if uncovered.is_empty() {
            ctx.traces.say(&format!(
                "criteria: AC1..AC{} of #{} each named by a test of PR {}, pointed to one or declared untestable",
                criteria.len(),
                ctx.state.task.number,
                pr.reference()
            ));
            return Ok(Verdict::Continue);
        }
        let found = findings(&criteria, &uncovered);
        ask_repair(
            self.gh.as_ref(),
            ctx,
            &pr,
            &Refusal {
                tag: "criteria",
                gate: "the criteria gate",
                lead: "Acceptance criteria without a test: name each in the test that proves \
                       it (`// AC2: …`); one an existing test or a rule under `tests/rules/` \
                       already proves is pointed to in the PR body \
                       (`AC2: covered by <path>[::<test>] — <why>`), never edited; one no test \
                       can prove is declared there (`AC2: not testable here — <reason>`):",
            },
            &found,
        )
        .await?;
        Err(Halt::Halted(format!(
            "acceptance criteria of #{} without a test in PR {} — name each in the test that \
             proves it (`// ACk: …`), point the PR body to an existing one (`ACk: {} <path> — \
             <why>`) or declare it there (`ACk: {} — <reason>`):\n  - {}",
            ctx.state.task.number,
            pr.reference(),
            acceptance::COVERED,
            acceptance::UNTESTABLE,
            found.join("\n  - ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::common::{labels, review};
    use harness_core::domain::Issue;
    use harness_core::domain::Named;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use std::collections::HashMap;

    const BODY: &str = "## Scope\n\nbranch: feat/list\n\n## Business Goal\n\ng\n\n\
                        ## Acceptance Criteria\n\n- given PJs, when opened, then listed\n\
                        - given none, when opened, then empty\n- the count shows\n";

    fn ctx(body: &str, dry_run: bool) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            Loop {
                task: Named {
                    number: "34".to_string(),
                    title: "List".to_string(),
                    body: body.to_string(),
                },
                ..Loop::default()
            },
            Logbook::null(),
        )
    }

    fn github(state: &str, files: &[(&str, &str)], pr_body: &str) -> FakeGitHub {
        FakeGitHub {
            prs: vec![(
                "feat/list".to_string(),
                Pr {
                    num: "101".to_string(),
                    head: "feat/list".to_string(),
                    state: state.to_string(),
                    ..Pr::default()
                },
            )],
            pr_files: HashMap::from([(
                "101".to_string(),
                files
                    .iter()
                    .map(|(path, _)| (*path).to_string())
                    .chain(["README.md".to_string()])
                    .collect(),
            )]),
            files: files
                .iter()
                .map(|(path, text)| {
                    (
                        ((*path).to_string(), "feat/list".to_string()),
                        (*text).to_string(),
                    )
                })
                .chain([(
                    ("README.md".to_string(), "feat/list".to_string()),
                    "AC3 named in a doc".to_string(),
                )])
                .collect(),
            issues: vec![Issue {
                number: 101,
                body: pr_body.to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        }
    }

    async fn judged(gh: FakeGitHub, body: &str) -> (Outcome<Verdict>, Vec<Wrote>) {
        let gh = Rc::new(gh);
        let gate = CriteriaTested {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        };
        let verdict = gate.verify(&ctx(body, false)).await;
        (verdict, gh.writes())
    }

    #[tokio::test]
    async fn every_criterion_named_or_declared_lets_the_round_through() {
        let gh = github(
            "OPEN",
            &[
                ("crates/a/tests/list.rs", "// AC1: listed\n// AC2: empty"),
                ("apps/a/count.ts", "describe(\"count\") // AC3"),
            ],
            "",
        );
        let (verdict, writes) = judged(gh, BODY).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)), "{verdict:?}");
        assert!(writes.is_empty(), "{writes:?}");
        let declared = github(
            "OPEN",
            &[("crates/a/tests/list.rs", "// AC1, AC2")],
            "## Plan\nAC3: not testable here — the count is drawn by the browser",
        );
        let (verdict, _) = judged(declared, BODY).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)), "{verdict:?}");
    }

    #[tokio::test]
    async fn a_criterion_no_test_names_fails_the_round_with_a_blocking_review() {
        let gh = github(
            "OPEN",
            &[
                ("crates/a/tests/list.rs", "// AC1: listed"),
                ("crates/a/src/lib.rs", "// AC2 named in shipped code only"),
            ],
            "AC3: not testable here",
        );
        let (verdict, writes) = judged(gh, BODY).await;
        let Err(Halt::Halted(reason)) = verdict else {
            panic!("halted: {verdict:?}");
        };
        for part in [
            "#34",
            "PR #101",
            "AC2 (given none, when opened, then empty)",
            "AC3 (the count shows)",
            "not testable here",
        ] {
            assert!(reason.contains(part), "{part} in {reason}");
        }
        assert!(!reason.contains("AC1 ("), "{reason}");
        let review_body = writes
            .iter()
            .find_map(|write| match write {
                Wrote::Comment(101, body) => Some(body.clone()),
                _ => None,
            })
            .expect("a review on the PR");
        assert!(review_body.contains("the criteria gate"), "{review_body}");
        assert!(review_body.contains("AC2 ("), "{review_body}");
        assert_eq!(
            review::status(&review_body),
            review::Status::Blocking { fixes: 0 }
        );
        assert!(writes.contains(&Wrote::Label(101, labels::PR_FIX.to_string())));
        assert!(writes.contains(&Wrote::Label(34, labels::REVIEW_PENDING.to_string())));
    }

    #[tokio::test]
    async fn nothing_is_judged_without_criteria_a_pr_or_an_open_one() {
        let bare = github("OPEN", &[], "");
        let without = BODY.replace("## Acceptance Criteria", "## Notes");
        let (verdict, writes) = judged(bare, &without).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)));
        assert!(writes.is_empty(), "{writes:?}");
        let no_branch = BODY.replace("branch: feat/list", "");
        let (verdict, _) = judged(github("OPEN", &[], ""), &no_branch).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)));
        let (verdict, _) = judged(FakeGitHub::default(), BODY).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)), "no PR yet");
        let (verdict, writes) = judged(github("MERGED", &[], ""), BODY).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)), "past judging");
        assert!(writes.is_empty(), "{writes:?}");
    }

    #[tokio::test]
    async fn a_dry_run_judges_nothing() {
        let gh = Rc::new(github("OPEN", &[], ""));
        let gate = CriteriaTested {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
        };
        let verdict = gate.verify(&ctx(BODY, true)).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)));
        assert!(gh.writes().is_empty(), "nothing written");
        assert!(gate.purpose().contains("AC1..ACn"));
    }

    #[tokio::test]
    async fn a_failed_read_judges_nothing_and_writes_nothing() {
        let files = [("crates/a/tests/list.rs", "// AC1")];
        let mut no_files = github("OPEN", &files, "");
        no_files.pr_files.clear();
        let mut no_body = github("OPEN", &files, "");
        no_body.issues.clear();
        let unreadable_file = github("OPEN", &files, "");
        let unreadable_file = FakeGitHub {
            unreadable_files: vec!["crates/a/tests/list.rs".to_string()],
            ..unreadable_file
        };
        let unreadable_pointer = FakeGitHub {
            unreadable_files: vec!["tests/old.rs".to_string()],
            ..github(
                "OPEN",
                &files,
                "AC2, AC3: covered by tests/old.rs — proven before",
            )
        };
        let mut bad_number = github("OPEN", &files, "");
        bad_number.prs[0].1.num = "x".to_string();
        bad_number.pr_files = HashMap::from([("x".to_string(), Vec::new())]);
        for gh in [
            no_files,
            no_body,
            unreadable_file,
            unreadable_pointer,
            bad_number,
        ] {
            let (verdict, writes) = judged(gh, BODY).await;
            assert!(matches!(verdict, Ok(Verdict::Continue)), "{verdict:?}");
            assert!(writes.is_empty(), "{writes:?}");
        }
    }

    struct Capture(std::cell::RefCell<Vec<String>>);

    impl harness_core::traces::Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    #[tokio::test]
    async fn a_failed_read_is_said_in_the_journal() {
        let gh = FakeGitHub {
            unreadable_files: vec!["crates/a/tests/list.rs".to_string()],
            ..github("OPEN", &[("crates/a/tests/list.rs", "// AC1")], "")
        };
        let gate = CriteriaTested { gh: Rc::new(gh) };
        let mut context = ctx(BODY, false);
        let capture = Rc::new(Capture(std::cell::RefCell::default()));
        context.traces = Logbook::new(
            Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
            harness_core::traces::Verbosity::Normal,
        );
        let verdict = gate.verify(&context).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)), "{verdict:?}");
        let said = capture.0.borrow().join("\n");
        assert!(
            said.contains(
                "criteria: `crates/a/tests/list.rs` at feat/list of PR #101 cannot be read — not judged"
            ),
            "{said}"
        );
    }

    #[tokio::test]
    async fn names_the_base_already_held_prove_nothing() {
        let old = "#[test]\nfn old() {}\n// AC1, AC2, AC3: an earlier task";
        let mut gh = github(
            "OPEN",
            &[("crates/a/tests/list.rs", &format!("{old}\n// a new line"))],
            "",
        );
        gh.prs[0].1.base = "main".to_string();
        gh.files.insert(
            ("crates/a/tests/list.rs".to_string(), "main".to_string()),
            old.to_string(),
        );
        let (verdict, _) = judged(gh, BODY).await;
        let Err(Halt::Halted(reason)) = verdict else {
            panic!("halted: {verdict:?}");
        };
        for part in ["AC1 (", "AC2 (", "AC3 ("] {
            assert!(reason.contains(part), "{part} in {reason}");
        }
    }

    #[tokio::test]
    async fn a_criterion_an_existing_test_proves_is_pointed_to() {
        let mut gh = github(
            "OPEN",
            &[("crates/a/tests/list.rs", "// AC1")],
            "AC2: covered by `tests/old.rs::lists_none` — proven before\n\
             AC3: covered by tests/rules/R103.spec.ts — the rule runs it",
        );
        for (path, text) in [
            ("tests/old.rs", "#[test]\nfn lists_none() {}"),
            ("tests/rules/R103.spec.ts", "test('R103', () => {})"),
        ] {
            gh.files.insert(
                (path.to_string(), "feat/list".to_string()),
                text.to_string(),
            );
        }
        let (verdict, writes) = judged(gh, BODY).await;
        assert!(matches!(verdict, Ok(Verdict::Continue)), "{verdict:?}");
        assert!(writes.is_empty(), "{writes:?}");
        let missing = github(
            "OPEN",
            &[("crates/a/tests/list.rs", "// AC1, AC2")],
            "AC3: covered by tests/gone.rs — it was there",
        );
        let (verdict, _) = judged(missing, BODY).await;
        let Err(Halt::Halted(reason)) = verdict else {
            panic!("halted: {verdict:?}");
        };
        assert!(reason.contains("AC3 (the count shows)"), "{reason}");
        assert!(reason.contains("covered by <path>"), "{reason}");
    }

    #[test]
    fn a_finding_names_the_criterion_and_its_words() {
        let criteria = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            findings(&criteria, &[2, 9]),
            [
                "AC2 (b): named by no test this PR adds, pointed to no existing test, declared \
                 nowhere",
                "AC9 (): named by no test this PR adds, pointed to no existing test, declared \
                 nowhere"
            ]
        );
    }
}
