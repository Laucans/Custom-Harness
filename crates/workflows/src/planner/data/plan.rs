//! The planner's own item type, parsed via the shared extraction.
//!
//! The session is asked to answer with a JSON array and nothing else, but a
//! model's reply is text, not structure — [`crate::common::json_reply`] is
//! the boundary where that text either becomes a plan or a reason it
//! couldn't; this module only names what one milestone looks like.

use serde::Deserialize;

pub use crate::common::json_reply::ParseError;
use crate::common::sections;
use harness_architecture::vocabulary::Side;

/// One milestone the plan wants opened, in delivery order.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MilestoneItem {
    /// A short, specific title — becomes the milestone issue's title.
    pub title: String,
    /// A few sentences on what this milestone achieves — becomes its body.
    pub goal: String,
    /// The systems (bounded contexts) it works in — one, where possible.
    #[serde(default)]
    pub systems: Vec<String>,
    /// The versioned Concepts it defines or implements (`Risk@3`).
    #[serde(default)]
    pub concepts: Vec<String>,
    /// Whether any of its work is on the write side — a `DataCapability`
    /// that mutates, an aggregate shaped. Such a milestone's tasks run one at
    /// a time, each merged once its agent review is clean.
    #[serde(default)]
    pub writes: bool,
    /// The aggregates it shapes or writes (`PJ`, `Campagne`): what chains
    /// it after the milestones that touch one of them, and lets two
    /// milestones on different aggregates open together.
    #[serde(default)]
    pub aggregates: Vec<String>,
    /// The rules (`R3`) whose tests turn green when it lands, as
    /// `rules.json` names them: what `split` hands its tasks.
    #[serde(default)]
    pub rules: Vec<String>,
    /// The micro-frontends it builds, as the dossier names them
    /// (`SceneBoard`): what the plan's graph reads its consumption from.
    #[serde(default)]
    pub micro_uis: Vec<String>,
    /// The writes it adds, by id (`session.addScene`): what it produces.
    #[serde(default)]
    pub commands: Vec<String>,
}

impl MilestoneItem {
    /// Which side of the architecture the milestone's work is on.
    #[must_use]
    pub const fn side(&self) -> Side {
        if self.writes { Side::Write } else { Side::Read }
    }

    /// The body: the goal, then the `## Architecture` section.
    #[must_use]
    pub fn body(&self) -> String {
        let list = |items: &[String]| {
            let kept: Vec<&str> = items
                .iter()
                .map(|item| item.trim())
                .filter(|item| !item.is_empty())
                .collect();
            if kept.is_empty() {
                "-".to_string()
            } else {
                kept.join(", ")
            }
        };
        let body = format!(
            "{}\n\n## {}\n\nsystems: {}\nconcepts: {}\naggregates: {}\nmicro-uis: {}\ncommands: {}\nside: {}",
            self.goal.trim(),
            sections::heading_of(sections::ARCHITECTURE),
            list(&self.systems),
            list(&self.concepts),
            list(&self.aggregates),
            list(&self.micro_uis),
            list(&self.commands),
            self.side().label()
        );
        let rules = list(&self.rules);
        if rules == "-" {
            body
        } else {
            format!(
                "{body}\n\n## {}\n\n{rules}",
                sections::heading_of(sections::RULES)
            )
        }
    }

    /// The aggregates, trimmed and lower-cased: what two milestones compare.
    #[must_use]
    pub fn aggregate_keys(&self) -> Vec<String> {
        self.aggregates
            .iter()
            .map(|a| a.trim().to_lowercase())
            .filter(|a| !a.is_empty() && a != "-")
            .collect()
    }
}

/// The values of one `key:` line of a milestone's `## Architecture`
/// (`aggregates: PJ, Campagne`), trimmed, as written.
#[must_use]
pub fn list_of(body: &str, key: &str) -> Vec<String> {
    sections::parse(body)
        .get(sections::ARCHITECTURE)
        .map(|text| {
            text.lines()
                .filter_map(|line| line.trim().strip_prefix(key))
                .flat_map(|value| value.split(','))
                .map(|a| a.trim().trim_matches('`').to_string())
                .filter(|a| !a.is_empty() && a != "-")
                .collect()
        })
        .unwrap_or_default()
}

/// The aggregates a milestone's body names (`aggregates: PJ, Campagne`
/// under `## Architecture`), lower-cased — what an already open milestone
/// is compared on.
#[must_use]
pub fn aggregates_of(body: &str) -> Vec<String> {
    list_of(body, "aggregates:")
        .into_iter()
        .map(|a| a.to_lowercase())
        .collect()
}

/// Whether `text` names `word` whole — not as part of a longer word.
fn mentions(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// A milestone as the plan's graph reads it, from its issue body.
///
/// A body the plan wrote names its `micro-uis:` and `commands:`; one
/// written before the plan did is read from its text — the dossier's
/// components it names whole, the writes it quotes.
#[must_use]
pub fn graph_input(
    number: u64,
    title: &str,
    body: &str,
    components: &[harness_architecture::graph::Component],
    writes: &[String],
) -> harness_architecture::graph::Milestone {
    let text = body.split("\n## ").next().unwrap_or(body);
    let mut micro_uis = list_of(body, "micro-uis:");
    if micro_uis.is_empty() {
        micro_uis = components
            .iter()
            .filter(|c| mentions(text, &c.name))
            .map(|c| c.name.clone())
            .collect();
    }
    let mut commands = list_of(body, "commands:");
    if commands.is_empty() {
        commands = writes
            .iter()
            .filter(|id| text.contains(&format!("`{id}`")))
            .cloned()
            .collect();
    }
    harness_architecture::graph::Milestone {
        number,
        title: title.to_string(),
        systems: list_of(body, "systems:"),
        aggregates: list_of(body, "aggregates:"),
        concepts: list_of(body, "concepts:"),
        micro_uis,
        commands,
    }
}

/// Extracts the JSON array of milestones from a reply's text.
///
/// # Errors
/// A [`ParseError`] if no array-shaped substring can be found, or if what
/// was found does not deserialize into `Vec<MilestoneItem>`.
pub fn parse(text: &str) -> Result<Vec<MilestoneItem>, ParseError> {
    crate::common::json_reply::parse_array(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_json_array_parses() {
        let items = parse(r#"[{"title":"A","goal":"do A"}]"#).expect("parse");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "A");
        assert_eq!(items[0].goal, "do A");
        assert_eq!(items[0].systems, [] as [std::string::String; 0]);
        assert!(!items[0].writes, "read-side unless said otherwise");
    }

    #[test]
    fn the_body_carries_the_goal_and_the_architecture_section() {
        let item = MilestoneItem {
            title: "Risk".to_string(),
            goal: "Score every customer.".to_string(),
            systems: vec!["credit".to_string(), " ".to_string()],
            concepts: vec!["Risk@3".to_string()],
            writes: true,
            aggregates: vec!["Account".to_string(), " ".to_string()],
            rules: Vec::new(),
            micro_uis: vec!["RiskBadge".to_string()],
            commands: Vec::new(),
        };
        assert_eq!(
            item.body(),
            "Score every customer.\n\n## Architecture\n\nsystems: credit\nconcepts: Risk@3\n\
             aggregates: Account\nmicro-uis: RiskBadge\ncommands: -\nside: harness:write-side"
        );
        assert_eq!(item.aggregate_keys(), ["account"]);
        assert_eq!(aggregates_of(&item.body()), ["account"]);
        assert_eq!(list_of(&item.body(), "micro-uis:"), ["RiskBadge"]);
        assert!(
            list_of(&item.body(), "commands:").is_empty(),
            "a dash is nothing"
        );
        assert!(aggregates_of("no section").is_empty(), "expected nothing");
        let bare = MilestoneItem {
            writes: false,
            aggregates: Vec::new(),
            systems: Vec::new(),
            concepts: Vec::new(),
            ..item
        };
        assert!(bare.body().ends_with(
            "systems: -\nconcepts: -\naggregates: -\nmicro-uis: RiskBadge\ncommands: -\nside: harness:read-side"
        ));
        assert_eq!(bare.side(), Side::Read);
    }

    #[test]
    fn the_rules_a_milestone_turns_green_follow_its_architecture() {
        let items =
            parse(r#"[{"title": "Campaigns", "goal": "List them.", "rules": ["R3", " ", "R5"]}]"#)
                .expect("parse");
        let body = items[0].body();
        assert!(
            body.ends_with("side: harness:read-side\n\n## Rules\n\nR3, R5"),
            "{body}"
        );
        assert_eq!(
            sections::parse(&body)
                .get(sections::RULES)
                .map(String::as_str),
            Some("R3, R5")
        );
        let none = parse(r#"[{"title": "t", "goal": "g"}]"#).expect("parse");
        assert!(!none[0].body().contains("## Rules"), "no rule, no section");
    }

    #[test]
    fn an_empty_array_is_valid_and_yields_nothing_to_create() {
        assert_eq!(parse("[]").expect("parse"), Vec::new());
    }

    #[test]
    fn multiple_milestones_keep_their_delivery_order() {
        let items = parse(r#"[{"title":"A","goal":"a"},{"title":"B","goal":"b"}]"#).expect("parse");
        assert_eq!(items[0].title, "A");
        assert_eq!(items[1].title, "B");
    }
}
