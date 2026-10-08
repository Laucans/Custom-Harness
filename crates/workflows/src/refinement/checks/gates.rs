//! What a refinement step requires, and what makes it skip.
//!
//! Don't confuse with the **workflow** gate (tooling + label, verified once
//! before all, mounted by the launcher). These only make sense in the round's
//! sequence.
//!
//! Each gate receives the name of the stage it speaks about: names are written
//! in the table and descend as a field.

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::common::architecture::Declaration;
use crate::common::sections;
use crate::refinement::data::rounds;
use crate::refinement::data::state::RefinementState;

/// The factory for a section stage's `skip`: does this round write it?
pub struct SectionIsWanted {
    /// The section key this stage carries.
    pub key: String,
}

#[async_trait(?Send)]
impl Verification<RefinementState> for SectionIsWanted {
    async fn verify(&self, ctx: &Context<RefinementState>) -> Outcome<Verdict> {
        if ctx.state.wanted.contains(&self.key) {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!("{} — not in this round", self.key)))
    }
}

/// The router only runs from round 2 on, and with a `--context`.
pub struct RouterIsOff {
    /// The name of the router stage, to name it when skipping.
    pub router: String,
    /// `--context` was given.
    pub has_context: bool,
}

#[async_trait(?Send)]
impl Verification<RefinementState> for RouterIsOff {
    async fn verify(&self, ctx: &Context<RefinementState>) -> Outcome<Verdict> {
        if rounds::routed(ctx.state.round_no, self.has_context) {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(format!(
            "{} — this round writes a fixed set of sections",
            self.router
        )))
    }
}

/// What the router must have obtained: the sections to reopen.
///
/// The "judge" half of the old `router_named_sections`; the "write" half
/// (`state.wanted = …`) lives in
/// [`action::actions::RecordWantedSections`](crate::refinement::action::actions::RecordWantedSections),
/// a local action slipped after the session — same split as
/// `dev_loop::checks::gates`.
pub struct RouterNamedSections {
    /// The name of the router stage, under which its response is stored in
    /// `ctx.results`.
    pub router: String,
}

#[async_trait(?Send)]
impl Verification<RefinementState> for RouterNamedSections {
    async fn verify(&self, ctx: &Context<RefinementState>) -> Outcome<Verdict> {
        // A dry-run runs no one: `state.wanted` stays what the preflight set
        // (empty, for a routed round), and that's not an error.
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        if !ctx.results.contains_key(&self.router) {
            // The step was skipped — latent today, refinement never filters
            // its stages.
            return Ok(Verdict::Continue);
        }
        if ctx.state.wanted.is_empty() {
            return Err(Halt::Failed(
                "the router named no section to reopen, so this round would \
                 write nothing — re-run with a --context that names what to \
                 rework, or without --context to rewrite all five sections"
                    .to_string(),
            ));
        }
        Ok(Verdict::Continue)
    }
}

/// A technical section of a Rust unit names its place under `crates/`.
///
/// The stack rule is the one a session negotiates away most readily in a
/// TypeScript host — "the code is TypeScript today", a waiver, a question
/// left to the human. The `## Architecture` section already settled it: a
/// Capability, a `DataCapability`, a persisted query, an invariant, a
/// migration or infrastructure is a crate, so the design and the plan that
/// never write `crates/<something>` did not build that unit.
pub struct SectionSitsInTheLayout {
    /// The section key this stage carries.
    pub key: String,
}

#[async_trait(?Send)]
impl Verification<RefinementState> for SectionSitsInTheLayout {
    async fn verify(&self, ctx: &Context<RefinementState>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(reply) = ctx.results.get(&self.key) else {
            return Ok(Verdict::Continue);
        };
        let architecture = ctx
            .state
            .found
            .get(sections::ARCHITECTURE)
            .map_or("", String::as_str);
        layout_violation(&self.key, architecture, &reply.text)
            .map_or(Ok(Verdict::Continue), |why| Err(Halt::Failed(why)))
    }
}

/// Why `text`, written for the section `key`, does not build the declared unit.
///
/// `None` when it does, when the section is not a technical one, or when the
/// unit the `## Architecture` section declares is not a Rust one.
#[must_use]
pub fn layout_violation(key: &str, architecture: &str, text: &str) -> Option<String> {
    if key != "technical" && key != "technical-plan" {
        return None;
    }
    let declaration = Declaration::parse(architecture)?;
    if !declaration.unit.is_rust() || names_a_crate(text) {
        return None;
    }
    Some(format!(
        "{key}: the task declares a {} — a Rust unit, under crates/ — and the \
         section names no path under crates/<system>/. The stack rule binds in \
         a TypeScript host too: the unit is a crate, compiled to WebAssembly \
         if the host must load it. Write that design, not a waiver, a fallback \
         or a question for the human.",
        declaration.unit.key()
    ))
}

/// `crates/` followed by the first character of a folder name — not the
/// bare `crates/` of "no file under `crates/` is modified".
fn names_a_crate(text: &str) -> bool {
    text.split("crates/").skip(1).any(|rest| {
        rest.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// A dry-run writes nothing: it wrote the prompts and stops there.
pub struct NothingIsWritten;

#[async_trait(?Send)]
impl Verification<RefinementState> for NothingIsWritten {
    async fn verify(&self, ctx: &Context<RefinementState>) -> Outcome<Verdict> {
        if !ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip("dry run — nothing written".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    const ROUTER: &str = "router";

    fn ctx(dry_run: bool) -> Context<RefinementState> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            RefinementState::default(),
            Logbook::null(),
        )
    }

    fn router_gate(has_context: bool) -> RouterIsOff {
        RouterIsOff {
            router: ROUTER.to_string(),
            has_context,
        }
    }

    const CAPABILITY: &str =
        "unit: capability\nsystem: bestiary\nconcept: Actors24Entry@1\nside: harness:read-side";

    #[test]
    fn a_technical_section_of_a_rust_unit_must_name_its_crate() {
        let waiver = "The decoder stays in `src/core` as the shell's local implementation; \
                      no file under `crates/` is modified.";
        let why = layout_violation("technical", CAPABILITY, waiver).expect("a violation");
        assert!(why.contains("capability"), "{why}");
        assert!(why.contains("WebAssembly"), "{why}");
        assert!(layout_violation("technical-plan", CAPABILITY, waiver).is_some());
        let crate_plan = "Step 1 creates `crates/bestiary/capabilities/actors24-entry/` with its \
                          capability.json, built to wasm by wasm-pack.";
        assert_eq!(layout_violation("technical", CAPABILITY, crate_plan), None);
    }

    #[test]
    fn the_layout_rule_spares_business_sections_and_non_rust_units() {
        assert_eq!(
            layout_violation("business-rules", CAPABILITY, "nothing"),
            None
        );
        let micro_ui = "unit: micro-ui\nsystem: bestiary\nside: harness:read-side";
        assert_eq!(
            layout_violation("technical", micro_ui, "apps/bestiary/palette"),
            None
        );
        assert_eq!(layout_violation("technical", "", "nothing"), None);
    }

    #[tokio::test]
    async fn a_section_not_wanted_this_round_is_skipped() {
        let gate = SectionIsWanted {
            key: "technical".to_string(),
        };
        assert!(matches!(
            gate.verify(&ctx(false)).await.expect("verdict"),
            Verdict::Skip(_)
        ));
    }

    #[tokio::test]
    async fn a_wanted_section_runs() {
        let mut context = ctx(false);
        context.state.wanted = vec!["technical".to_string()];
        let gate = SectionIsWanted {
            key: "technical".to_string(),
        };
        assert_eq!(
            gate.verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn the_router_is_off_on_the_first_round() {
        let mut context = ctx(false);
        context.state.round_no = 1;
        assert!(matches!(
            router_gate(true).verify(&context).await.expect("verdict"),
            Verdict::Skip(_)
        ));
    }

    #[tokio::test]
    async fn the_router_runs_from_round_two_with_context() {
        let mut context = ctx(false);
        context.state.round_no = 2;
        assert_eq!(
            router_gate(true).verify(&context).await.expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_router_that_named_nothing_stops_the_round() {
        use harness_core::domain::Spend;
        use harness_core::ports::agent::Reply;
        let mut context = ctx(false);
        context.results.insert(
            ROUTER.to_string(),
            Reply {
                text: String::new(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        let err = RouterNamedSections {
            router: ROUTER.to_string(),
        }
        .verify(&context)
        .await
        .expect_err("doit s'arrêter");
        assert!(err.reason().contains("--context"));
    }

    #[tokio::test]
    async fn a_dry_run_never_fails_on_an_empty_router_answer() {
        let context = ctx(true);
        assert_eq!(
            RouterNamedSections {
                router: ROUTER.to_string(),
            }
            .verify(&context)
            .await
            .expect("verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing() {
        assert!(matches!(
            NothingIsWritten.verify(&ctx(true)).await.expect("verdict"),
            Verdict::Skip(_)
        ));
    }
}
