//! What the test-debt's paid stage requires before it costs anything.
//!
//! One gate: the issue names a Rust source file. An issue whose title names
//! nothing — a human renamed it, or opened one by hand — is a `Skip`, not an
//! error: there is nothing wrong with the harness, only nothing to ask.

use async_trait::async_trait;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::test_debt::data::state::DebtState;

/// The issue names the file whose mutants are to be caught.
pub struct NamesAFile;

#[async_trait(?Send)]
impl Verification<DebtState> for NamesAFile {
    fn purpose(&self) -> String {
        "the issue names the Rust source file whose mutants no test catches".to_string()
    }

    async fn verify(&self, ctx: &Context<DebtState>) -> Outcome<Verdict> {
        if ctx.state.file.is_none() {
            return Ok(Verdict::Skip(
                "the issue's title names no Rust source file — nothing to ask".to_string(),
            ));
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(file: Option<&str>) -> Context<DebtState> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            DebtState {
                file: file.map(str::to_string),
                ..DebtState::default()
            },
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn a_named_file_opens_the_paid_stage_and_none_skips_it() {
        assert_eq!(
            NamesAFile
                .verify(&ctx(Some("src/lib.rs")))
                .await
                .expect("verdict"),
            Verdict::Continue
        );
        assert!(matches!(
            NamesAFile
                .verify(&ctx(None))
                .await
                .expect("a skip, not an error"),
            Verdict::Skip(_)
        ));
    }
}
