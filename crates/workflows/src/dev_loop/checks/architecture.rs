//! Does the checkout still follow the agent-native architecture once `code`
//! is done? Two gates on the round, both deterministic and both local.
//!
//! The reading and the rules are [`harness_architecture::tree`]; these gates
//! only read the checkout through the `Disk` port, hand the rules the task's
//! own `## Architecture` declaration, and stop the round with the list.
//!
//! [`ArchitectureHolds`] checks every rule a diff is reviewed against, and that
//! the unit the task declared is there, on the side it said.
//! [`ConceptsDocumented`] checks that every Concept a Capability implements is
//! documented: meaning is what is shared between systems, and a Concept nobody
//! wrote down is a copy nobody can check.
//!
//! The CI gates of the repository run the same rules; these are the loop's
//! own reading, so a round that drifted stops here with the list rather than
//! at the next pull request's checks. A checkout without `contracts/` is not
//! under the architecture: both gates say so and let the round through.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_architecture::tree::{Tree, undocumented_concepts, violations};
use harness_architecture::vocabulary::Declaration;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};
use harness_core::ports::shell::disk::Disk;

use crate::common::sections;
use crate::dev_loop::data::state::Loop;

/// The task's own declaration, read from its body.
fn declaration_of(ctx: &Context<Loop>) -> Option<Declaration> {
    sections::parse(&ctx.state.task.body)
        .get(sections::ARCHITECTURE)
        .and_then(|text| Declaration::parse(text))
}

/// The round kept the architecture: see the module.
pub struct ArchitectureHolds {
    /// What reads the checkout.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl Verification<Loop> for ArchitectureHolds {
    fn purpose(&self) -> String {
        "static read of the checkout: every Capability stays isolated, every manifest names its system, every Micro-UI needs Capabilities of its own system, and the unit the task declared is there on the side it said".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        if !tree.under_architecture {
            ctx.traces
                .say("architecture: no contracts/ in this checkout — not under the architecture, gate skipped");
            return Ok(Verdict::Continue);
        }
        let found = violations(&tree, declaration_of(ctx).as_ref());
        if found.is_empty() {
            ctx.traces.say(&format!(
                "architecture holds: {} Capability crate(s), {} Micro-UI(s), {} DataCapabilit(ies)",
                tree.capabilities.len(),
                tree.micro_uis.len(),
                tree.data_capabilities.len()
            ));
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "the architecture no longer holds after #{} — fix these before anything else builds \
             on them (the repository's gates check the same rules):\n  - {}",
            ctx.state.task.number,
            found.join("\n  - ")
        )))
    }
}

/// Every Concept a Capability implements is documented: see the module.
pub struct ConceptsDocumented {
    /// What reads the checkout.
    pub disk: Rc<dyn Disk>,
    /// The checkout's root.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl Verification<Loop> for ConceptsDocumented {
    fn purpose(&self) -> String {
        "static read of the checkout: every Concept a Capability implements has its concepts/<Name>/v<n>/concept.json".to_string()
    }

    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let tree = Tree::read(self.disk.as_ref(), &self.root);
        if !tree.under_architecture {
            return Ok(Verdict::Continue);
        }
        let found = undocumented_concepts(&tree);
        if found.is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "a Capability implements a Concept nobody documented — write \
             concepts/<Concept>/v<n>/concept.json (contract A) and its fixtures first:\n  - {}",
            found.join("\n  - ")
        )))
    }
}
