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

/// The label a read-side task carries, on its issue and in its declaration.
pub const READ_SIDE: &str = "harness:read-side";

/// The label a write-side task carries, on its issue and in its declaration.
pub const WRITE_SIDE: &str = "harness:write-side";

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
    /// One aggregate's structure: its `aggregate.json`, its invariants, the
    /// migration that shapes its tables — what its `DataCapabilities` then
    /// write through. Held while it is built: two tasks on one aggregate
    /// run in turn.
    Aggregate,
    /// The plumbing itself: `DataQueue`, `DataGuard`, Resolver, the Data layer.
    Infrastructure,
    /// Rust that touches no store: contract types, shared helpers, CI and
    /// build tooling. Read side — nothing it ships mutates data.
    Tooling,
}

impl Unit {
    /// Every unit, in the order the architecture presents them.
    pub const ALL: [Self; 9] = [
        Self::Capability,
        Self::MicroUi,
        Self::Concept,
        Self::DataCapability,
        Self::PersistedQuery,
        Self::Composition,
        Self::Aggregate,
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
            Self::Aggregate => "aggregate",
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
    /// `MicroUI`, `micro_ui` and `micro-ui` are one unit. `invariant` and
    /// `migration`, the units an aggregate's task used to be cut into, read
    /// as `aggregate`: an issue opened before still has a unit.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let folded: String = text
            .trim()
            .to_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        if folded == "invariant" || folded == "migration" {
            return Some(Self::Aggregate);
        }
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
            Self::Read => READ_SIDE,
            Self::Write => WRITE_SIDE,
        }
    }
}

/// The layer of the architecture a task is built in — the order a
/// milestone is built in.
///
/// Each layer reads the one before it: a Concept is read by everything, the
/// data layer exists before anything reads or calls it, the contract fixes
/// the Capabilities each implement one action in parallel against the data,
/// and the UI composes them last. The layers order a plan for a human to
/// read; what a task waits on is the graph `split` draws from what each
/// one produces and needs, never the layer before its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// `concept`: a definition everything else reads.
    Concept,
    /// `aggregate`, `data-capability` (insert included), `infrastructure`,
    /// `tooling`: the data, built first.
    Data,
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
            Self::DataCapability | Self::Aggregate | Self::Infrastructure | Self::Tooling => {
                Layer::Data
            }
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
    /// The aggregate a unit of the data layer writes or changes the structure
    /// of (`PJ`), as its manifest's `target.aggregate` names it: what two
    /// such tasks must not hold at once.
    pub aggregate: Option<String>,
    /// The ids this task delivers, as the manifests will name them: an
    /// aggregate (`PJ`), a `DataCapability` (`campagne.ajouterPj`), a
    /// Capability, a Micro-UI (`Pjs`), a screen. What later tasks wait on.
    pub produces: Vec<String>,
    /// The ids this task builds on — what it reads, triggers or composes.
    /// `split` chains it on the task that produces each; one no task of the
    /// plan produces is read as already there.
    pub needs: Vec<String>,
    /// How much one session is trusted to hold: `strong`, `mid` or `small`
    /// — what the slice was cut for, so the SPEC is calibrated the same way.
    pub profile: Option<String>,
    /// Where the unit runs: `hostless` when no server does — a plugin, a
    /// static app — and the unit is compiled to WebAssembly for its host.
    /// Absent, the product's server.
    pub host: Option<String>,
}

impl Declaration {
    /// The aggregate this task holds while it runs, if it changes its
    /// structure: an invariant, a migration, or a `DataCapability` that is
    /// not a pure addition. A pure insert only adds a folder; a reader holds
    /// nothing. Two tasks holding the same aggregate run one after the other.
    #[must_use]
    pub fn held_aggregate(&self) -> Option<&str> {
        let changes_structure = match self.unit {
            Unit::Aggregate => true,
            Unit::DataCapability => self.side() == Side::Write,
            _ => false,
        };
        changes_structure
            .then_some(self.aggregate.as_deref())
            .flatten()
    }

    /// The side the architecture assigns to this unit.
    ///
    /// Everything on the read side runs without review; a `DataCapability` is
    /// read-side only as an insert that touches nothing existing — the
    /// architecture's "pure addition" — and every other effect, every
    /// aggregate and piece of store plumbing is the write side.
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
            Unit::Aggregate | Unit::Infrastructure => Side::Write,
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
        if let Some(aggregate) = &self.aggregate {
            lines.push(format!("aggregate: {aggregate}"));
        }
        if !self.produces.is_empty() {
            lines.push(format!("produces: {}", self.produces.join(", ")));
        }
        if !self.needs.is_empty() {
            lines.push(format!("needs: {}", self.needs.join(", ")));
        }
        if let Some(profile) = &self.profile {
            lines.push(format!("profile: {profile}"));
        }
        if let Some(host) = &self.host {
            lines.push(format!("host: {host}"));
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
        let mut aggregate = None;
        let mut produces = Vec::new();
        let mut needs = Vec::new();
        let mut profile = None;
        let mut host = None;
        let list = |value: &str| -> Vec<String> {
            value
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty() && *id != "-")
                .map(ToString::to_string)
                .collect()
        };
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
                "aggregate" => {
                    aggregate = (!value.is_empty() && value != "-").then(|| value.to_string());
                }
                "produces" => produces = list(value),
                "needs" => needs = list(value),
                "profile" => {
                    profile = (!value.is_empty() && value != "-").then(|| value.to_lowercase());
                }
                "host" => {
                    host = (!value.is_empty() && value != "-").then(|| value.to_lowercase());
                }
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
            aggregate,
            produces,
            needs,
            profile,
            host,
        })
    }

    /// True when no server runs the unit: it is compiled for its host, and
    /// no server dependency line or server folder is asked of it.
    #[must_use]
    pub fn is_hostless(&self) -> bool {
        self.host.as_deref() == Some("hostless")
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
            aggregate: None,
            produces: Vec::new(),
            needs: Vec::new(),
            profile: None,
            host: None,
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
        // What an aggregate's task used to be cut into still reads as one.
        assert_eq!(Unit::parse("invariant"), Some(Unit::Aggregate));
        assert_eq!(Unit::parse("Migration"), Some(Unit::Aggregate));
    }

    #[test]
    fn the_layers_are_built_in_order_and_a_pure_insert_is_still_data() {
        assert!(Layer::Concept < Layer::Data);
        assert!(Layer::Data < Layer::Capability);
        assert!(Layer::Capability < Layer::Ui);
        assert_eq!(Unit::DataCapability.layer(), Layer::Data);
        assert_eq!(Unit::Aggregate.layer(), Layer::Data);
        assert_eq!(Unit::Tooling.layer(), Layer::Data);
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
        for unit in [Unit::Aggregate, Unit::Infrastructure] {
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
            aggregate: Some("Account".to_string()),
            produces: vec!["credit.raiseLimit".to_string()],
            needs: vec!["Account".to_string(), "Risk@3".to_string()],
            profile: Some("small".to_string()),
            host: Some("hostless".to_string()),
        };
        let text = written.render();
        assert_eq!(
            text,
            "unit: data-capability\nsystem: credit\neffect: update\n\
             touches: Account.creditLimit, Account.tags\naggregate: Account\n\
             produces: credit.raiseLimit\nneeds: Account, Risk@3\nprofile: small\nhost: hostless\nside: harness:write-side"
        );
        assert_eq!(Declaration::parse(&text), Some(written.clone()));
        assert!(written.is_hostless());
        for none in ["host: -", "host:"] {
            let parsed = Declaration::parse(&format!("unit: capability\nsystem: credit\n{none}"))
                .expect("parsed");
            assert_eq!(parsed.host, None, "{none}: no host");
            assert!(!parsed.is_hostless());
        }
        // Held while it runs: it changes the aggregate's data. A pure insert and a reader hold nothing.
        assert_eq!(written.held_aggregate(), Some("Account"));
        let pure = Declaration {
            effect: Some(Effect::Insert),
            touches: Vec::new(),
            ..written.clone()
        };
        assert_eq!(
            pure.held_aggregate(),
            None,
            "a pure addition adds a folder, holds nothing"
        );
        let migration = Declaration {
            unit: Unit::Aggregate,
            effect: None,
            touches: Vec::new(),
            ..written
        };
        assert_eq!(migration.held_aggregate(), Some("Account"));
        let mut reader = capability();
        reader.aggregate = Some("Account".to_string());
        assert_eq!(reader.held_aggregate(), None, "a reader holds nothing");
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
        assert_eq!(Side::Read.label(), READ_SIDE);
        assert_eq!(Side::Write.label(), WRITE_SIDE);
    }
}
