//! Opens the tasks the plan lacks, under the stopped task's milestone, for
//! the loop's stop handler and for `decide`: one opener, whoever approves.
//!
//! `ForgeDesMondes`' #219, 2026-10-25: `decide` answered "the fixtures task
//! is approved: it becomes a write-side task of milestone #28 that blocks
//! #219" four rounds running, and opened nothing — #219 reran and stopped
//! again each time. An approval is now this opener's work, as a stop's
//! declaration is: the issue opened (or found, declared before), linked
//! under the milestone, and the waiting task blocked on it.

use harness_core::domain::Outcome;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Logbook;

use crate::common::labels;
use crate::dev_loop::data::missing;
use crate::split::action::publish::body_and_labels;
use crate::split::data::plan::TaskItem;

/// What became of the slices: the tasks to wait on, and those refused
/// because a closed sibling built their branch already.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The tasks to wait on, in order, each once.
    pub on: Vec<u64>,
    /// The slices not opened, a closed task having built their branch:
    /// title, why — in words the session acts on.
    pub built: Vec<(String, String)>,
}

/// Opens each slice under `milestone`, or finds it there.
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
    for item in items {
        let task = if let Some(same) = others().find(|sibling| missing::is_the_same(sibling, item))
        {
            log.say(&format!(
                "#{number}: the missing task is #{} already — parked on it",
                same.number
            ));
            same.number
        } else if let Some(built) = others().find(|sibling| missing::built_already(sibling, item)) {
            let why = format!(
                "#{} built `{}` already (closed): build on it, or declare a new branch",
                built.number,
                item.branch.trim()
            );
            log.warn(&format!(
                "#{number}: \"{}\" is not opened — {why}",
                item.title
            ));
            found.built.push((item.title.clone(), why));
            continue;
        } else {
            let opened = open(gh, number, milestone, item).await?;
            log.say(&format!(
                "#{number}: opened #{opened} for the missing task ({}) under milestone \
                 #{milestone}",
                item.title
            ));
            opened
        };
        if !found.on.contains(&task) {
            found.on.push(task);
        }
    }
    Ok(found)
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

/// Opens one missing task: split's body and labels, its `id:`, and a line
/// back to the task that waits on it.
async fn open(gh: &dyn GitHub, number: u64, milestone: u64, item: &TaskItem) -> Outcome<u64> {
    let mut item = item.clone();
    item.brief = format!(
        "{}\n\nOpened by the harness for #{number}, whose session stopped on it: #{number} \
         waits on this task.",
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
    Ok(opened)
}
