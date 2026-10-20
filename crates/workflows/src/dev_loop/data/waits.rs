//! What a stop's reason names that another issue owns — what a task waits
//! for, through a GitHub blocker, instead of asking a decision.
//!
//! `ForgeDesMondes`, 2026-10-18: #214 and #215 stopped 23 times each on a
//! producer that had not landed (`origin/main_agent has no crates/session/`,
//! #20's), and every stop paid a code session and a decide session that
//! answered "keep waiting". The reason named what to wait for every time:
//! an issue (`#20`), a milestone's branch (`milestone/20-sessions`), an id
//! of the plan's graph (`session.listScenes`, `Session`) or a path under a
//! system's crate (`crates/session/`). Read here, pure; the stop handler
//! checks on the board that what is named has not landed.

use harness_architecture::graph::Graph;

use crate::common::branching;

/// The issues a stop's reason names, in the order named, each once.
///
/// `#N`; a `milestone/N-…` branch; the producer of an id the graph knows,
/// named whole; the producers a path names — the milestone working in the
/// system whose crate it is under, the producer of an id one of its words
/// is. Without a graph, the first two only.
#[must_use]
pub fn named(reason: &str, graph: Option<&Graph>) -> Vec<u64> {
    let mut found = Vec::new();
    for token in reason.split_whitespace().map(trim_punctuation) {
        if let Some(number) = issue_reference(token) {
            keep(&mut found, number);
        } else if let Some(number) = token
            .find("milestone/")
            .and_then(|at| token.get(at..))
            .and_then(branching::milestone_number)
        {
            keep(&mut found, number);
        } else if let Some(graph) = graph {
            if let Some(producer) = graph.ids.get(token).and_then(|id| id.producer) {
                keep(&mut found, producer);
            } else if token.contains('/') {
                for producer in in_path(token, graph) {
                    keep(&mut found, producer);
                }
            }
        }
    }
    found
}

/// Pushes `number` unless it is there already.
fn keep(found: &mut Vec<u64>, number: u64) {
    if !found.contains(&number) {
        found.push(number);
    }
}

/// `#20`, or `#20's` — the number after the hash.
fn issue_reference(token: &str) -> Option<u64> {
    let digits: String = token
        .strip_prefix('#')?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// A token without the quotes, brackets and punctuation around it; what an
/// id or a path is made of stays (`session.listScenes`, `crates/session/`,
/// `milestone/20-sessions`, `Risk@3`).
fn trim_punctuation(token: &str) -> &str {
    token.trim_matches(|c: char| !(c.is_alphanumeric() || matches!(c, '#' | '/' | '@' | '_' | '-')))
}

/// The producers a path names, case aside: the producer of an id one of
/// its words is (`crates/data/migrations/0004_entity.sql`, `Entity`'s)
/// first — an id is the more specific — then the milestone working in the
/// system whose crate the path is under: the segment right after `crates/`
/// (`crates/session/`), never any word of the path. `ForgeDesMondes`'
/// #156 stopped on `crates/dataguard/aggregates/Campaign/aggregate.json`
/// and waited on #30, the campaign home, for the word `campaign`.
fn in_path(path: &str, graph: &Graph) -> Vec<u64> {
    let words: Vec<String> = path
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let ids = words.iter().filter_map(|word| {
        graph
            .ids
            .iter()
            .find(|(id, _)| id.to_lowercase() == *word)
            .and_then(|(_, id)| id.producer)
    });
    let under_crates = path
        .to_lowercase()
        .split('/')
        .skip_while(|segment| *segment != "crates")
        .nth(1)
        .map(str::to_string);
    let systems = under_crates.into_iter().filter_map(|crate_| {
        graph
            .milestones
            .iter()
            .find(|node| {
                node.systems
                    .iter()
                    .any(|system| system.to_lowercase() == crate_)
            })
            .map(|node| node.number)
    });
    ids.chain(systems).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn graph() -> Graph {
        Graph::parse(
            r#"{"version":1,"milestones":[
              {"number":18,"title":"notes","systems":["note"],"produces":["Entity"],"consumes":[],"depends_on":[]},
              {"number":20,"title":"sessions","systems":["session"],"produces":["session.listScenes","Session"],"consumes":[],"depends_on":[]},
              {"number":28,"title":"media","systems":["media"],"produces":[],"consumes":["session.listScenes"],"depends_on":[20]},
              {"number":12,"title":"data layer","systems":["data"],"produces":[],"consumes":[],"depends_on":[]}
            ],"ids":{
              "Entity":{"kind":"aggregate","producer":18,"state":"planned","version":1},
              "session.listScenes":{"kind":"capability","producer":20,"state":"planned","version":1},
              "Session":{"kind":"aggregate","producer":20,"state":"planned","version":1},
              "nobody.knows":{"kind":"capability","producer":null,"state":"planned","version":1}
            }}"#,
        )
        .expect("a graph")
    }

    #[test]
    fn an_issue_or_a_milestone_branch_is_named_without_a_graph() {
        assert_eq!(
            named(
                "AGENT_LOOP_STOP: wait for #20 — #20's crate is not there (#215)",
                None
            ),
            [20, 215],
            "each once, in order"
        );
        assert_eq!(
            named(
                "origin/milestone/20-sessions is not merged; `milestone/18-notes` neither",
                None
            ),
            [20, 18]
        );
        assert!(
            named("which payment provider? (/code)", None).is_empty(),
            "named('which payment provider? (/code)', None) should be empty"
        );
        assert!(
            named("#abc and # 20 name nothing", None).is_empty(),
            "named('#abc and # 20 name nothing', None) should be empty"
        );
    }

    #[test]
    fn an_id_or_a_path_is_named_through_the_graph() {
        let graph = graph();
        assert_eq!(
            named("origin/main_agent has no crates/session/", Some(&graph)),
            [20],
            "the system's crate"
        );
        assert_eq!(
            named(
                "missing crates/data/migrations/0004_entity.sql",
                Some(&graph)
            ),
            [18, 12],
            "the aggregate's producer before the system its crate is under"
        );
        assert_eq!(
            named(
                "`session.listScenes` is not built, nor is Session.",
                Some(&graph)
            ),
            [20],
            "an id named whole, trailing punctuation aside"
        );
        assert!(
            named("nobody.knows is planned by no milestone", Some(&graph)).is_empty(),
            "an id without a producer names no one"
        );
        assert!(
            named("origin/main_agent has no crates/session/", None).is_empty(),
            "no graph, no system"
        );
        assert!(
            named("the session store is slow", Some(&graph)).is_empty(),
            "a bare word is not an id"
        );
        assert_eq!(
            named(
                "missing crates/dataguard/aggregates/Session/aggregate.json on milestone/28",
                Some(&graph)
            ),
            [20, 28],
            "a word of the path is an id, never a system: `dataguard` is no system's crate"
        );
        assert!(
            named("the media folder apps/media/library is empty", Some(&graph)).is_empty(),
            "a system's name anywhere but right under crates/ names no milestone"
        );
    }

    proptest! {
        #[test]
        fn reading_any_reason_never_panics(reason in ".{0,200}") {
            let _ = named(&reason, Some(&graph()));
            let _ = named(&reason, None);
        }
    }
}
