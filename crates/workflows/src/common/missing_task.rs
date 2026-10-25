//! Opens the tasks the plan lacks, under the stopped task's milestone, for
//! the loop's stop handler and for `decide`: one opener, whoever approves.
//!
//! `ForgeDesMondes`' #219, 2026-10-25: `decide` answered "the fixtures task
//! is approved: it becomes a write-side task of milestone #28 that blocks
//! #219" four rounds running, and opened nothing — #219 reran and stopped
//! again each time. An approval is now this opener's work, as a stop's
//! declaration is: the issue opened (or found, declared before), linked
//! under the milestone, and the waiting task blocked on it.
//!
//! **Siblings are chained** ([`edges`]): a slice that `needs` an id another
//! slice of the same declaration `produces` — or a task opened earlier for
//! the same waiting task — is blocked on it (`ForgeDesMondes`' #568 needed
//! `session.updateSceneNotes` from #567, both from one stop, and halted on
//! `ArchitectureHolds` until it was chained by hand).

use harness_core::domain::{Issue, Outcome};
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::{Event, Logbook};

use crate::common::{labels, native_milestone};
use crate::dev_loop::data::missing;
use crate::split::action::publish::body_and_labels;
use crate::split::data::plan::TaskItem;

/// What became of the slices: the tasks to wait on, those refused because
/// a closed sibling built their branch already, and the chains written
/// between them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The tasks to wait on, in order, each once.
    pub on: Vec<u64>,
    /// The tasks opened by this call, in order — `on` less those found.
    pub opened: Vec<u64>,
    /// The slices not opened, a closed task having built their branch:
    /// title, why — in words the session acts on.
    pub built: Vec<(String, String)>,
    /// The blockers written between siblings: the blocked task, its
    /// blocker, the id one needs and the other builds.
    pub chained: Vec<(u64, u64, String)>,
}

/// The line every task this opener opens carries, up to the waiting
/// task's number.
const OPENED_FOR: &str = "Opened by the harness for #";

/// The task an issue was opened for by this opener — the one that waits on
/// it — when its body carries the opener's line.
#[must_use]
pub fn opened_for(issue: &Issue) -> Option<u64> {
    let (_, after) = issue.body.split_once(OPENED_FOR)?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// An id as two declarations compare it: case, backticks and a version
/// (`@1`) aside.
fn id_key(id: &str) -> String {
    let id = id.trim().trim_matches('`').trim();
    id.split('@').next().unwrap_or(id).trim().to_lowercase()
}

/// Whether one of `ids` is `need` ([`id_key`]).
fn builds<'a>(mut ids: impl Iterator<Item = &'a String>, need: &str) -> bool {
    ids.any(|id| id_key(id) == need)
}

/// A chain between two siblings: the blocked task, its blocker, the id.
pub type Edge = (u64, u64, String);

/// The chains a declaration needs, then the edges left out.
///
/// `(blocked, blocker, id)` for each slice that `needs` an id another slice
/// `produces` — else a kin task (opened earlier for the same waiting task)
/// does. `slices` are the declared slices with the task each became; `kin` the
/// other tasks opened for `waiting`, with what they build. Ids compare case
/// aside (as split's `graph_blockers` does). Never the waiting task, never a
/// slice on itself, one edge per pair; an edge that would close a cycle
/// among those chosen is left out, and returned second.
#[must_use]
pub fn edges(
    slices: &[(u64, &TaskItem)],
    kin: &[(u64, Vec<String>)],
    waiting: u64,
) -> (Vec<Edge>, Vec<Edge>) {
    let mut chosen: Vec<Edge> = Vec::new();
    let mut cycles: Vec<Edge> = Vec::new();
    for (blocked, item) in slices {
        if *blocked == waiting {
            continue;
        }
        for need in &item.needs {
            let need_key = id_key(need);
            if need_key.is_empty() || need_key == "-" {
                continue;
            }
            let producer = slices
                .iter()
                .find(|(number, other)| {
                    number != blocked && builds(other.produces.iter(), &need_key)
                })
                .map(|(number, _)| *number)
                .or_else(|| {
                    kin.iter()
                        .find(|(number, ids)| number != blocked && builds(ids.iter(), &need_key))
                        .map(|(number, _)| *number)
                });
            let Some(blocker) = producer.filter(|b| *b != waiting) else {
                continue;
            };
            if chosen.iter().any(|(a, b, _)| a == blocked && *b == blocker) {
                continue;
            }
            let edge = (*blocked, blocker, need.trim().trim_matches('`').to_string());
            if reaches(&chosen, blocker, *blocked) {
                cycles.push(edge);
            } else {
                chosen.push(edge);
            }
        }
    }
    (chosen, cycles)
}

/// Whether `from` waits, through `edges`, on `to`.
fn reaches(edges: &[Edge], from: u64, to: u64) -> bool {
    let mut seen: Vec<u64> = vec![from];
    let mut next = vec![from];
    while let Some(at) = next.pop() {
        if at == to {
            return true;
        }
        for (blocked, blocker, _) in edges {
            if *blocked == at && !seen.contains(blocker) {
                seen.push(*blocker);
                next.push(*blocker);
            }
        }
    }
    false
}

/// Opens each slice under `milestone`, or finds it there, and blocks
/// `number` on each as soon as it is opened or found.
///
/// Found: an open agent task already ([`missing::is_the_same`]). A slice
/// whose branch a closed sibling built ([`missing::built_already`]) is
/// never opened again: one branch is one feature, and the slice goes back
/// to the session.
///
/// # Errors
/// The GitHub reads and writes'.
pub async fn open_or_find(
    gh: &dyn GitHub,
    number: u64,
    milestone: u64,
    items: &[TaskItem],
    log: &Logbook,
) -> Outcome<Found> {
    let mut found = Found::default();
    if items.is_empty() {
        return Ok(found);
    }
    let siblings = gh.sub_issues(milestone).await?;
    let others = || siblings.iter().filter(|sibling| sibling.number != number);
    let mut blockers: Vec<u64> = gh
        .blocked_by(number)
        .await?
        .iter()
        .map(|b| b.number)
        .collect();
    let mut slices: Vec<(u64, &TaskItem)> = Vec::new();
    for item in items {
        let task = if let Some(same) = others().find(|sibling| missing::is_the_same(sibling, item))
        {
            log.say(&format!(
                "#{number}: the missing task is #{} already — parked on it",
                same.number
            ));
            same.number
        } else if let Some(built) = others().find(|sibling| missing::built_already(sibling, item)) {
            // Closed as not needed (its own session's evidence): a new
            // declaration must show it missing, or nothing tells the two apart.
            let why = if built.not_planned {
                format!(
                    "#{} was closed as not needed (its last comment says why): build on, or \
                     declare it under a new branch with the probe that shows it missing",
                    built.number
                )
            } else {
                format!(
                    "#{} built `{}` already (closed): build on it, or declare a new branch",
                    built.number,
                    item.branch.trim()
                )
            };
            log.warn(&format!(
                "#{number}: \"{}\" is not opened — {why}",
                item.title
            ));
            found.built.push((item.title.clone(), why));
            continue;
        } else {
            let opened = open(gh, number, milestone, item, log).await?;
            log.say(&format!(
                "#{number}: opened #{opened} for the missing task ({}) under milestone \
                 #{milestone}",
                item.title
            ));
            found.opened.push(opened);
            opened
        };
        // Parked at once: a publish that fails on the next slice still
        // leaves the task waiting on those opened before it.
        if !blockers.contains(&task) {
            gh.add_blocked_by(number, task).await?;
            blockers.push(task);
        }
        slices.push((task, item));
        if !found.on.contains(&task) {
            found.on.push(task);
        }
    }
    found.chained = chain(gh, number, &siblings, &slices, log).await?;
    Ok(found)
}

/// Writes the blockers between siblings ([`edges`]), each once, with a
/// `TaskWaits` each; a cycle left out is warned.
async fn chain(
    gh: &dyn GitHub,
    waiting: u64,
    siblings: &[Issue],
    slices: &[(u64, &TaskItem)],
    log: &Logbook,
) -> Outcome<Vec<Edge>> {
    let kin: Vec<(u64, Vec<String>)> = siblings
        .iter()
        .filter(|sibling| {
            sibling.is_open()
                && sibling.has(labels::AGENT)
                && opened_for(sibling) == Some(waiting)
                && !slices.iter().any(|(number, _)| *number == sibling.number)
        })
        .map(|sibling| (sibling.number, missing::produced(sibling)))
        .collect();
    let (chosen, cycles) = edges(slices, &kin, waiting);
    for (blocked, blocker, id) in &cycles {
        log.warn(&format!(
            "#{waiting}: #{blocked} is not chained on #{blocker} (`{id}`) — it would close a cycle"
        ));
    }
    for (blocked, blocker, id) in &chosen {
        block_on(gh, *blocked, &[*blocker]).await?;
        log.event(&Event::TaskWaits {
            task: *blocked,
            on: *blocker,
            why: format!("needs `{id}`, which #{blocker} builds"),
        });
    }
    Ok(chosen)
}

/// Blocks `number` on every issue of `on`, each blocker written once.
///
/// # Errors
/// The GitHub reads and writes'.
pub async fn block_on(gh: &dyn GitHub, number: u64, on: &[u64]) -> Outcome<()> {
    let blocked_by = gh.blocked_by(number).await?;
    for blocker in on {
        if !blocked_by.iter().any(|known| known.number == *blocker) {
            gh.add_blocked_by(number, *blocker).await?;
        }
    }
    Ok(())
}

/// What the session is told of the declared tasks the harness did not
/// open, one line each — its title and why — or nothing when all were.
#[must_use]
pub fn refusal_note(unopened: &[(String, String)]) -> String {
    if unopened.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = unopened
        .iter()
        .map(|(title, why)| {
            format!(
                "- \"{title}\": {}",
                why.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        })
        .collect();
    format!(
        "**Declared tasks the harness did not open** — declare each again, fixed, in one \
         `AGENT_LOOP_STOP: tasks [...]` (the unit one of: {}), or build on without it if it \
         is not missing after all:\n\n{}",
        missing::unit_keys(),
        lines.join("\n")
    )
}

/// What every opened task starts with: a slice declared from a journey's
/// failure may be built already, by a sibling or a catch-up.
pub const STILL_NEEDED: &str = "Start by checking, on the live code — and on the rules runtime \
     when it is a fixture or a server read — that this is still missing. When it is not, stop \
     with `AGENT_LOOP_STOP: not-needed — <the evidence>`.";

/// Opens one missing task: split's body and labels, its `id:`, and a line
/// back to the task that waits on it; linked under `milestone` and put on
/// its native milestone (a refusal of the latter only journalled).
async fn open(
    gh: &dyn GitHub,
    number: u64,
    milestone: u64,
    item: &TaskItem,
    log: &Logbook,
) -> Outcome<u64> {
    let mut item = item.clone();
    item.brief = format!(
        "{}\n\n{OPENED_FOR}{number}, whose session stopped on it: #{number} waits on this \
         task.\n\n{STILL_NEEDED}",
        item.brief.trim()
    );
    let declaration = item.declaration();
    let (body, labels) = body_and_labels(&item, &declaration, labels::AGENT, declaration.side());
    let rendered = declaration.render();
    let body = missing::id(&item).map_or_else(
        || body.clone(),
        |id| body.replacen(&rendered, &missing::with_id(&rendered, id), 1),
    );
    let opened = gh.create_issue(&item.title, &body, &labels).await?;
    gh.create_sub_issue_link(milestone, opened).await?;
    native_milestone::place_or_warn(gh, milestone, opened, log).await;
    Ok(opened)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use harness_core::traces::{Sink, Verbosity};

    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::dev_loop::data::missing::Asked;

    /// The stop #219's session wrote at 13:17, 2026-10-25: #567 and #568.
    const STOP_219_ROUND_6: &str = include_str!("../../tests/fixtures/stop-219-round-6.txt");

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

    fn told() -> (Rc<Told>, Logbook) {
        let sink = Rc::new(Told::default());
        let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, Verbosity::Normal);
        (sink, log)
    }

    fn round_6() -> Vec<TaskItem> {
        missing::asked_all(STOP_219_ROUND_6)
            .expect("a list")
            .into_iter()
            .map(|asked| match asked {
                Asked::Task(item) => *item,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    fn item(produces: &[&str], needs: &[&str]) -> TaskItem {
        serde_json::from_value(serde_json::json!({
            "title": "t", "branch": "feat/t", "brief": "b", "unit": "micro-ui",
            "produces": produces, "needs": needs,
        }))
        .expect("an item")
    }

    #[tokio::test]
    async fn the_stop_of_13_17_chains_the_scene_board_on_the_notes_capability() {
        let gh = FakeGitHub::default();
        gh.next_issue_number.set(566);
        let (sink, log) = told();
        let items = round_6();
        let found = open_or_find(&gh, 219, 28, &items, &log)
            .await
            .expect("opened");
        assert_eq!(found.on, [567, 568]);
        assert_eq!(found.opened, [567, 568]);
        assert_eq!(
            found.chained,
            [(568, 567, "session.updateSceneNotes".to_string())]
        );
        let links: Vec<Wrote> = gh
            .writes()
            .into_iter()
            .filter(|w| matches!(w, Wrote::BlockedByLink(..)))
            .collect();
        assert_eq!(
            links,
            [
                Wrote::BlockedByLink(219, 567),
                Wrote::BlockedByLink(219, 568),
                Wrote::BlockedByLink(568, 567)
            ]
        );
        let events = sink.0.borrow().1.clone();
        assert_eq!(
            events,
            [Event::TaskWaits {
                task: 568,
                on: 567,
                why: "needs `session.updateSceneNotes`, which #567 builds".to_string(),
            }]
        );
        // Every opened task names its waiting task and checks it is needed.
        let bodies: Vec<String> = gh
            .writes()
            .into_iter()
            .filter_map(|w| match w {
                Wrote::CreatedIssue(_, body, _) => Some(body),
                _ => None,
            })
            .collect();
        assert_eq!(bodies.len(), 2);
        for body in &bodies {
            assert!(body.contains(STILL_NEEDED), "{body}");
            let issue = Issue {
                body: body.clone(),
                ..Issue::default()
            };
            assert_eq!(opened_for(&issue), Some(219));
        }
    }

    #[tokio::test]
    async fn a_slice_needing_what_a_kin_task_builds_is_chained_on_it_and_found_ones_are_not_opened()
    {
        // #567 opened by an earlier stop of #219; the next stop declares the
        // Micro-UI only.
        let kin = Issue {
            number: 567,
            state: "open".to_string(),
            title: "DataCapability session.updateSceneNotes".to_string(),
            labels: vec![labels::AGENT.to_string()],
            body: "## Scope\n\nOpened by the harness for #219, whose session stopped on it.\n\n\
                   ## Architecture\n\nunit: data-capability\nid: session.updateSceneNotes"
                .to_string(),
            ..Issue::default()
        };
        let other = Issue {
            number: 570,
            body: kin
                .body
                .replace("#219", "#300")
                .replace("updateSceneNotes", "x"),
            ..kin.clone()
        };
        let gh = FakeGitHub {
            subs: vec![(28, vec![kin, other])],
            ..FakeGitHub::default()
        };
        gh.next_issue_number.set(567);
        let (_, log) = told();
        let items: Vec<TaskItem> = round_6().into_iter().skip(1).collect();
        let found = open_or_find(&gh, 219, 28, &items, &log)
            .await
            .expect("opened");
        assert_eq!(found.on, [568]);
        assert_eq!(
            found.chained,
            [(568, 567, "session.updateSceneNotes".to_string())]
        );
        // Both declared again: both found, chained still, nothing opened.
        let (_, log) = told();
        let gh = FakeGitHub {
            subs: gh.subs.clone(),
            ..FakeGitHub::default()
        };
        let all = round_6();
        let found = open_or_find(&gh, 219, 28, &all[..1], &log)
            .await
            .expect("found");
        assert_eq!(
            (found.on.clone(), found.opened.clone()),
            (vec![567], vec![])
        );
        assert!(found.chained.is_empty(), "{:?}", found.chained);
    }

    #[tokio::test]
    async fn only_an_open_agent_task_opened_for_the_same_task_is_kin() {
        let kin = |number: u64, state: &str, label: &str, waiting: u64| Issue {
            number,
            state: state.to_string(),
            title: format!("Kin {number}"),
            labels: vec![label.to_string()],
            body: format!(
                "Opened by the harness for #{waiting}, whose session stopped on it.\n\n\
                 ## Architecture\n\nunit: data-capability\nid: session.updateSceneNotes"
            ),
            ..Issue::default()
        };
        let gh = FakeGitHub {
            subs: vec![(
                28,
                vec![
                    kin(561, "closed", labels::AGENT, 219),
                    kin(562, "open", labels::HUMAN, 219),
                    kin(563, "open", labels::AGENT, 300),
                ],
            )],
            ..FakeGitHub::default()
        };
        gh.next_issue_number.set(567);
        let (_, log) = told();
        let items: Vec<TaskItem> = round_6().into_iter().skip(1).collect();
        let found = open_or_find(&gh, 219, 28, &items, &log)
            .await
            .expect("opened");
        assert_eq!(found.opened, [568]);
        assert_eq!(found.chained, [], "{found:?}");
    }

    #[test]
    fn edges_chain_needs_on_produces_case_aside_never_the_waiting_task_nor_a_cycle() {
        let notes = item(&["session.updateSceneNotes"], &["session.addScene"]);
        let board = item(
            &["SceneBoard"],
            &["SESSION.updatescenenotes@1", "session.getSession"],
        );
        let (chosen, cycles) = edges(&[(568, &board), (567, &notes)], &[], 219);
        assert_eq!(
            chosen,
            [(568, 567, "SESSION.updatescenenotes@1".to_string())]
        );
        assert!(cycles.is_empty(), "{cycles:?}");
        // A kin task builds what no slice does.
        let kin = vec![(560, vec!["session.addScene".to_string()])];
        let (chosen, _) = edges(&[(567, &notes)], &kin, 219);
        assert_eq!(chosen, [(567, 560, "session.addScene".to_string())]);
        // A slice beats a kin task for the same id.
        let adder = item(&["session.addScene"], &[]);
        let (chosen, _) = edges(&[(567, &notes), (569, &adder)], &kin, 219);
        assert_eq!(chosen, [(567, 569, "session.addScene".to_string())]);
        // Never the waiting task, as blocker or blocked; never on itself.
        let (chosen, _) = edges(&[(219, &board), (567, &notes)], &[], 219);
        assert!(chosen.is_empty(), "{chosen:?}");
        let kin_waiting = vec![(219, vec!["session.addScene".to_string()])];
        assert!(
            edges(&[(567, &notes)], &kin_waiting, 219).0.is_empty(),
            "{:?}",
            edges(&[(567, &notes)], &kin_waiting, 219).0
        );
        let selfish = item(&["a.b"], &["a.b"]);
        assert!(
            edges(&[(5, &selfish)], &[], 219).0.is_empty(),
            "{:?}",
            edges(&[(5, &selfish)], &[], 219).0
        );
        // A cycle: the second edge is left out and named.
        let a = item(&["a.x"], &["b.y"]);
        let b = item(&["b.y"], &["a.x"]);
        let (chosen, cycles) = edges(&[(1, &a), (2, &b)], &[], 219);
        assert_eq!(chosen, [(1, 2, "b.y".to_string())]);
        assert_eq!(cycles, [(2, 1, "a.x".to_string())]);
        // Through a third: 1 -> 2 -> 3 -> 1 closes on the last.
        let c1 = item(&["c.one"], &["c.two"]);
        let c2 = item(&["c.two"], &["c.three"]);
        let c3 = item(&["c.three"], &["c.one"]);
        let (chosen, cycles) = edges(&[(1, &c1), (2, &c2), (3, &c3)], &[], 9);
        assert_eq!(chosen.len(), 2);
        assert_eq!(cycles, [(3, 1, "c.one".to_string())]);
        // A dash or nothing names no id, whatever a sibling declares.
        let dashed = item(&["-", ""], &[]);
        let needs_dash = item(&[], &["-"]);
        let needs_nothing = item(&[], &[" "]);
        assert!(
            edges(
                &[(1, &needs_dash), (3, &needs_nothing), (2, &dashed)],
                &[],
                9
            )
            .0
            .is_empty(),
            "a dash is no id"
        );
        // One slice on two producers, two slices on one: an edge each.
        let pa = item(&["p.a"], &[]);
        let qb = item(&["q.b"], &[]);
        let both = item(&[], &["p.a", "q.b"]);
        let (chosen, _) = edges(&[(1, &both), (2, &pa), (3, &qb)], &[], 9);
        assert_eq!(
            chosen,
            [(1, 2, "p.a".to_string()), (1, 3, "q.b".to_string())]
        );
        let only_a = item(&[], &["p.a"]);
        let (chosen, _) = edges(&[(1, &only_a), (4, &only_a), (2, &pa)], &[], 9);
        assert_eq!(
            chosen,
            [(1, 2, "p.a".to_string()), (4, 2, "p.a".to_string())]
        );
        // Two ids between one pair: one edge.
        let two = item(&["p.a", "p.b"], &[]);
        let needs_two = item(&[], &["p.a", "`p.b`", " - ", ""]);
        let (chosen, _) = edges(&[(1, &needs_two), (2, &two)], &[], 9);
        assert_eq!(chosen, [(1, 2, "p.a".to_string())]);
    }

    #[tokio::test]
    async fn a_publish_that_fails_halfway_leaves_the_task_parked_on_what_it_opened() {
        // GitHub refuses the second blocker: the first is written already.
        let gh = FakeGitHub {
            refused_blockers: vec![568],
            ..FakeGitHub::default()
        };
        gh.next_issue_number.set(566);
        let (_, log) = told();
        assert!(open_or_find(&gh, 219, 28, &round_6(), &log).await.is_err());
        let links: Vec<Wrote> = gh
            .writes()
            .into_iter()
            .filter(|w| matches!(w, Wrote::BlockedByLink(..)))
            .collect();
        assert_eq!(links, [Wrote::BlockedByLink(219, 567)]);
        // A blocker known already is not written twice.
        let known = Issue {
            number: 567,
            state: "open".to_string(),
            ..Issue::default()
        };
        let gh = FakeGitHub {
            subs: vec![(
                28,
                vec![Issue {
                    number: 219,
                    blocked_by: vec![known],
                    ..Issue::default()
                }],
            )],
            ..FakeGitHub::default()
        };
        gh.next_issue_number.set(566);
        let (_, log) = told();
        let found = open_or_find(&gh, 219, 28, &round_6(), &log)
            .await
            .expect("opened");
        assert_eq!(found.on, [567, 568]);
        let parked: Vec<Wrote> = gh
            .writes()
            .into_iter()
            .filter(|w| matches!(w, Wrote::BlockedByLink(219, _)))
            .collect();
        assert_eq!(parked, [Wrote::BlockedByLink(219, 568)]);
    }

    #[test]
    fn the_waiting_task_is_read_off_the_opener_s_line_only() {
        let issue = |body: &str| Issue {
            body: body.to_string(),
            ..Issue::default()
        };
        assert_eq!(
            opened_for(&issue("x\n\nOpened by the harness for #219, whose")),
            Some(219)
        );
        assert_eq!(
            opened_for(&issue("Opened by the harness for #21")),
            Some(21)
        );
        assert_eq!(opened_for(&issue("Opened by the harness for #x")), None);
        assert_eq!(opened_for(&issue("opened for #219")), None);
    }

    #[tokio::test]
    async fn a_cycle_is_warned_and_not_written() {
        let a: TaskItem = serde_json::from_value(serde_json::json!({
            "title": "A", "branch": "feat/a", "brief": "b", "unit": "data-capability",
            "system": "s", "produces": ["s.a"], "needs": ["s.b"], "goal": "g",
            "acceptance": ["given a, when b, then c"],
        }))
        .expect("a");
        let mut b = a.clone();
        b.title = "B".to_string();
        b.branch = "feat/b".to_string();
        b.produces = vec!["s.b".to_string()];
        b.needs = vec!["s.a".to_string()];
        let gh = FakeGitHub::default();
        let (sink, log) = told();
        let found = open_or_find(&gh, 9, 3, &[a, b], &log).await.expect("ok");
        assert_eq!(found.chained, [(1, 2, "s.b".to_string())]);
        let lines = sink.0.borrow().0.clone();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#9: #2 is not chained on #1 (`s.a`) — it would close a cycle")),
            "{lines:?}"
        );
    }

    proptest::proptest! {
        #[test]
        fn edges_never_close_a_cycle_touch_the_waiting_task_or_repeat_a_pair(
            declared in proptest::collection::vec(
                (
                    1u64..8,
                    proptest::collection::vec(0usize..6, 0..3),
                    proptest::collection::vec(0usize..6, 0..3),
                ),
                0..7,
            ),
            kin in proptest::collection::vec(
                (1u64..8, proptest::collection::vec(0usize..6, 0..3)),
                0..3,
            ),
            waiting in 1u64..8,
        ) {
            let ids = ["a.one", "a.two", "B.Three", "b.four@1", "`c.five`", "-"];
            let name = |picked: &[usize]| -> Vec<String> {
                picked.iter().map(|k| ids[*k].to_string()).collect()
            };
            let items: Vec<(u64, TaskItem)> = declared
                .iter()
                .map(|(number, produces, needs)| {
                    let mut slice = item(&[], &[]);
                    slice.produces = name(produces);
                    slice.needs = name(needs);
                    (*number, slice)
                })
                .collect();
            let slices: Vec<(u64, &TaskItem)> =
                items.iter().map(|(number, slice)| (*number, slice)).collect();
            let kin: Vec<(u64, Vec<String>)> = kin
                .iter()
                .map(|(number, produces)| (*number, name(produces)))
                .collect();
            let (chosen, cycles) = edges(&slices, &kin, waiting);
            for (k, (blocked, blocker, _)) in chosen.iter().enumerate() {
                proptest::prop_assert_ne!(blocked, blocker);
                proptest::prop_assert_ne!(*blocked, waiting);
                proptest::prop_assert_ne!(*blocker, waiting);
                proptest::prop_assert!(
                    !chosen[..k].iter().any(|(a, b, _)| a == blocked && b == blocker),
                    "{chosen:?}"
                );
                // No cycle: the blocker never waits back on the blocked.
                proptest::prop_assert!(!reaches(&chosen, *blocker, *blocked), "{chosen:?}");
            }
            for (blocked, blocker, _) in &cycles {
                proptest::prop_assert!(reaches(&chosen, *blocker, *blocked), "{cycles:?}");
            }
        }

        #[test]
        fn the_waiting_task_reads_back_and_any_body_never_panics(
            waiting in 0u64..1_000_000,
            body in "(?s).{0,200}",
        ) {
            let issue = |body: String| Issue { body, ..Issue::default() };
            let _ = opened_for(&issue(body.clone()));
            proptest::prop_assume!(!body.contains(OPENED_FOR));
            proptest::prop_assert_eq!(
                opened_for(&issue(format!("{body}\n\n{OPENED_FOR}{waiting}, whose session"))),
                Some(waiting)
            );
        }
    }
}
