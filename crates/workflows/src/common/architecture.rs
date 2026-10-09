//! The agent-native architecture's vocabulary, as an issue carries it.
//!
//! Every repository the plant works on follows one architecture (installed
//! by `init-repo` as `docs/ARCHITECTURE.md`): a read side of many
//! independent units — Capabilities, Micro-UIs, Concepts, persisted queries,
//! screen compositions — and a write side behind one `DataGuard` —
//! `DataCapabilities` that mutate, invariants, relations, migrations. A task
//! is one unit of it, and says which under its `## Architecture` section.
//!
//! This module is that section: the [`Unit`] a task builds, the
//! [`Declaration`] `split` writes and the loop reads back, and the one rule
//! that falls out of it — the task's [`Side`], which decides whether several
//! tasks may run in parallel and whether a human must merge the PR.
//!
//! Pure business logic: no I/O. The section is a few `key: value` lines, so
//! a human can write one by hand and a session can read it without a parser
//! worth the name.

use crate::common::labels;

/// What a task builds, in the architecture's own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// A read-only unit that collects, transforms or enriches data.
    Capability,
    /// A screen fragment that declares what it reads and what it can trigger.
    MicroUi,
    /// A versioned definition shared across systems, with conformance fixtures.
    Concept,
    /// A typed command behind the `DataGuard` — the only thing that writes.
    DataCapability,
    /// A declared, hashed query a Capability reads through.
    PersistedQuery,
    /// A frozen screen: Micro-UIs composed for one user goal.
    Composition,
    /// An integrity rule the `DataGuard` checks, or a relation's policy.
    Invariant,
    /// A schema change, expand/contract.
    Migration,
    /// The plumbing itself: `DataQueue`, `DataGuard`, Resolver, the Data layer.
    Infrastructure,
    /// Rust that touches no store: contract types, shared helpers, CI and
    /// build tooling. Read side — nothing it ships mutates data.
    Tooling,
}

impl Unit {
    /// Every unit, in the order the architecture presents them.
    pub const ALL: [Self; 10] = [
        Self::Capability,
        Self::MicroUi,
        Self::Concept,
        Self::DataCapability,
        Self::PersistedQuery,
        Self::Composition,
        Self::Invariant,
        Self::Migration,
        Self::Infrastructure,
        Self::Tooling,
    ];

    /// The spelling a session is asked for, and the one written in the body.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Capability => "capability",
            Self::MicroUi => "micro-ui",
            Self::Concept => "concept",
            Self::DataCapability => "data-capability",
            Self::PersistedQuery => "persisted-query",
            Self::Composition => "composition",
            Self::Invariant => "invariant",
            Self::Migration => "migration",
            Self::Infrastructure => "infrastructure",
            Self::Tooling => "tooling",
        }
    }

    /// True for the units the stack builds in Rust, under `crates/` — all
    /// but a Micro-UI, a screen composition and a Concept's document.
    #[must_use]
    pub const fn is_rust(self) -> bool {
        !matches!(self, Self::MicroUi | Self::Composition | Self::Concept)
    }

    /// From a spelling, forgiving case, underscores and a missing dash:
    /// `MicroUI`, `micro_ui` and `micro-ui` are one unit.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let folded: String = text
            .trim()
            .to_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        Self::ALL.into_iter().find(|unit| {
            let key: String = unit.key().chars().filter(|c| *c != '-').collect();
            key == folded
        })
    }
}

/// What a `DataCapability` does to the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// A new row, a new identifier: the fast path when nothing existing is touched.
    Insert,
    /// Changes an existing row.
    Update,
    /// Removes a row — a soft delete, by the architecture's default.
    Delete,
    /// Inserts or updates: touches existing data by construction.
    Upsert,
}

impl Effect {
    /// The spelling written in the body and in the contract.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Upsert => "upsert",
        }
    }

    /// From a spelling, case-insensitive.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "insert" => Some(Self::Insert),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            "upsert" => Some(Self::Upsert),
            _ => None,
        }
    }
}

/// Which side of the frontier a task works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Reads only: runs in parallel with its peers, merges on its own.
    Read,
    /// Mutates existing data or the rules that guard it: a human merges.
    Write,
}

impl Side {
    /// The label that says it on the issue.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Read => labels::READ_SIDE,
            Self::Write => labels::WRITE_SIDE,
        }
    }
}

/// The layer of the architecture a task is built in — the order a
/// milestone is built in.
///
/// Each layer reads the one before it: a Concept is read by everything, the
/// data layer exists before anything reads or calls it, the contract fixes
/// what the Capabilities and the Micro-UIs exchange, the Capabilities each
/// implement one action in parallel against it, and the UI composes them
/// last. `split` chains a task on the layer before its own; inside a layer
/// nothing is chained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// `concept`: a definition everything else reads.
    Concept,
    /// `data-capability` (insert included), `invariant`, `migration`,
    /// `infrastructure`: built first, serialized behind the `DataGuard`.
    Data,
    /// `tooling`: the interface contracts and the types they exchange.
    Contract,
    /// `capability`, `persisted-query`: one action each, in parallel.
    Capability,
    /// `micro-ui`, `composition`: the screen, once every action exists.
    Ui,
}

impl Layer {
    /// The layer's name, as a trace says it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Concept => "concept",
            Self::Data => "data layer",
            Self::Contract => "contract",
            Self::Capability => "capability",
            Self::Ui => "ui",
        }
    }
}

impl Unit {
    /// The layer this unit is built in.
    #[must_use]
    pub const fn layer(self) -> Layer {
        match self {
            Self::Concept => Layer::Concept,
            Self::DataCapability | Self::Invariant | Self::Migration | Self::Infrastructure => {
                Layer::Data
            }
            Self::Tooling => Layer::Contract,
            Self::Capability | Self::PersistedQuery => Layer::Capability,
            Self::MicroUi | Self::Composition => Layer::Ui,
        }
    }
}

/// A task's place in the architecture — the `## Architecture` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// What the task builds.
    pub unit: Unit,
    /// The bounded context it belongs to: `credit`, `orders`, `platform`.
    pub system: String,
    /// The versioned Concept it implements or defines, if any: `Risk@3`.
    pub concept: Option<String>,
    /// For a `DataCapability`, what it does to the data.
    pub effect: Option<Effect>,
    /// For a `DataCapability`, the existing fields it touches: empty with an
    /// insert is what makes the fast path.
    pub touches: Vec<String>,
}

impl Declaration {
    /// The side the architecture assigns to this unit.
    ///
    /// Everything on the read side runs without review; a `DataCapability` is
    /// read-side only as an insert that touches nothing existing — the
    /// architecture's "pure addition" — and every other effect, every
    /// invariant, migration and piece of store plumbing is the write side.
    /// `Tooling` is Rust with no store in it, and reads.
    #[must_use]
    pub fn side(&self) -> Side {
        match self.unit {
            Unit::Capability
            | Unit::MicroUi
            | Unit::Concept
            | Unit::PersistedQuery
            | Unit::Composition
            | Unit::Tooling => Side::Read,
            Unit::Invariant | Unit::Migration | Unit::Infrastructure => Side::Write,
            Unit::DataCapability => {
                if self.effect == Some(Effect::Insert) && self.touches.is_empty() {
                    Side::Read
                } else {
                    Side::Write
                }
            }
        }
    }

    /// The section's text: one `key: value` line each, nothing else.
    #[must_use]
    pub fn render(&self) -> String {
        let mut lines = vec![
            format!("unit: {}", self.unit.key()),
            format!(
                "system: {}",
                if self.system.is_empty() {
                    "-"
                } else {
                    &self.system
                }
            ),
        ];
        if let Some(concept) = &self.concept {
            lines.push(format!("concept: {concept}"));
        }
        if let Some(effect) = self.effect {
            lines.push(format!("effect: {}", effect.key()));
            let touches = if self.touches.is_empty() {
                "-".to_string()
            } else {
                self.touches.join(", ")
            };
            lines.push(format!("touches: {touches}"));
        }
        lines.push(format!("side: {}", self.side().label()));
        lines.join("\n")
    }

    /// Reads a section back. `None` when it names no unit, or an unknown one
    /// — a body nobody structured, which the loop treats as read-side with a
    /// warning rather than refusing to run.
    #[must_use]
    pub fn parse(section: &str) -> Option<Self> {
        let mut unit = None;
        let mut system = String::new();
        let mut concept = None;
        let mut effect = None;
        let mut touches = Vec::new();
        for line in section.lines() {
            let line = line.trim().trim_start_matches(['-', '*']).trim();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim().trim_matches('`');
            match key
                .trim()
                .trim_matches(['*', '_', '`'])
                .trim()
                .to_lowercase()
                .as_str()
            {
                "unit" => unit = Unit::parse(value),
                // `-` is how a body says "no system" — infrastructure, a
                // cross-cutting unit — never a system named `-`.
                "system" => {
                    system = if value == "-" {
                        String::new()
                    } else {
                        value.to_string()
                    }
                }
                "concept" => {
                    concept = (!value.is_empty() && value != "-").then(|| value.to_string());
                }
                "effect" => effect = Effect::parse(value),
                "touches" => {
                    touches = value
                        .split(',')
                        .map(str::trim)
                        .filter(|field| !field.is_empty() && *field != "-")
                        .map(ToString::to_string)
                        .collect();
                }
                _ => {}
            }
        }
        Some(Self {
            unit: unit?,
            system,
            concept,
            effect,
            touches,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability() -> Declaration {
        Declaration {
            unit: Unit::Capability,
            system: "credit".to_string(),
            concept: Some("Risk@3".to_string()),
            effect: None,
            touches: Vec::new(),
        }
    }

    #[test]
    fn every_unit_round_trips_through_its_key_whatever_the_spelling() {
        for unit in Unit::ALL {
            assert_eq!(Unit::parse(unit.key()), Some(unit));
            assert_eq!(Unit::parse(&unit.key().to_uppercase()), Some(unit));
        }
        assert_eq!(Unit::parse("MicroUI"), Some(Unit::MicroUi));
        assert_eq!(Unit::parse("data_capability"), Some(Unit::DataCapability));
        assert_eq!(Unit::parse("Persisted Query"), Some(Unit::PersistedQuery));
        assert_eq!(Unit::parse("widget"), None);
    }

    #[test]
    fn the_layers_are_built_in_order_and_a_pure_insert_is_still_data() {
        assert!(Layer::Concept < Layer::Data);
        assert!(Layer::Data < Layer::Contract);
        assert!(Layer::Contract < Layer::Capability);
        assert!(Layer::Capability < Layer::Ui);
        assert_eq!(Unit::DataCapability.layer(), Layer::Data);
        assert_eq!(Unit::Tooling.layer(), Layer::Contract);
        assert_eq!(Unit::PersistedQuery.layer(), Layer::Capability);
        assert_eq!(Unit::Composition.layer(), Layer::Ui);
    }

    #[test]
    fn the_read_side_is_what_runs_alone_and_the_write_side_what_a_human_merges() {
        assert_eq!(capability().side(), Side::Read);
        for unit in [
            Unit::MicroUi,
            Unit::Concept,
            Unit::PersistedQuery,
            Unit::Composition,
            Unit::Tooling,
        ] {
            let declaration = Declaration {
                unit,
                ..capability()
            };
            assert_eq!(declaration.side(), Side::Read, "{unit:?}");
        }
        for unit in [Unit::Invariant, Unit::Migration, Unit::Infrastructure] {
            let declaration = Declaration {
                unit,
                ..capability()
            };
            assert_eq!(declaration.side(), Side::Write, "{unit:?}");
        }
    }

    #[test]
    fn a_data_capability_is_read_side_only_as_a_pure_insert() {
        let pure = Declaration {
            unit: Unit::DataCapability,
            effect: Some(Effect::Insert),
            touches: Vec::new(),
            ..capability()
        };
        assert_eq!(pure.side(), Side::Read);
        let touching = Declaration {
            touches: vec!["Account.creditLimit".to_string()],
            ..pure.clone()
        };
        assert_eq!(
            touching.side(),
            Side::Write,
            "an insert that touches existing data"
        );
        for effect in [Effect::Update, Effect::Delete, Effect::Upsert] {
            let mutating = Declaration {
                effect: Some(effect),
                ..pure.clone()
            };
            assert_eq!(mutating.side(), Side::Write, "{effect:?}");
        }
        let undeclared = Declaration {
            effect: None,
            ..pure
        };
        assert_eq!(
            undeclared.side(),
            Side::Write,
            "no effect declared: not a fast path"
        );
    }

    #[test]
    fn a_declaration_renders_to_lines_a_human_can_read_and_parses_back() {
        let written = Declaration {
            unit: Unit::DataCapability,
            system: "credit".to_string(),
            concept: None,
            effect: Some(Effect::Update),
            touches: vec![
                "Account.creditLimit".to_string(),
                "Account.tags".to_string(),
            ],
        };
        let text = written.render();
        assert_eq!(
            text,
            "unit: data-capability\nsystem: credit\neffect: update\n\
             touches: Account.creditLimit, Account.tags\nside: harness:write-side"
        );
        assert_eq!(Declaration::parse(&text), Some(written));
        let text = capability().render();
        assert!(text.contains("concept: Risk@3"));
        assert!(!text.contains("effect:"), "a Capability has no effect");
        assert!(text.ends_with("side: harness:read-side"));
        assert_eq!(Declaration::parse(&text), Some(capability()));
    }

    #[test]
    fn parsing_forgives_bullets_backticks_and_case_and_refuses_a_body_without_a_unit() {
        let hand_written = "- **Unit**: `Micro-UI`\n- System: Credit\n- Concept: -\n- touches: -";
        let parsed = Declaration::parse(hand_written).expect("a unit is named");
        assert_eq!(parsed.unit, Unit::MicroUi);
        assert_eq!(parsed.system, "Credit");
        assert_eq!(parsed.concept, None);
        assert_eq!(parsed.touches, [] as [std::string::String; 0]);
        assert_eq!(Declaration::parse("system: credit\nsome prose"), None);
    }

    #[test]
    fn a_dash_system_is_no_system_and_renders_back_as_a_dash() {
        let read = Declaration::parse("unit: infrastructure\nsystem: -\nside: harness:write-side")
            .expect("a unit");
        assert_eq!(read.system, "");
        assert!(read.render().contains("system: -"));
        assert_eq!(Declaration::parse(""), None);
    }

    #[test]
    fn the_side_names_its_label() {
        assert_eq!(Side::Read.label(), labels::READ_SIDE);
        assert_eq!(Side::Write.label(), labels::WRITE_SIDE);
    }
}
