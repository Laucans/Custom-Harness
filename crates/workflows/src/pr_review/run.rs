//! Ce qu'une revue établit avant de payer, et le verrou qu'elle tient.
//!
//! **Une revue sautée est un succès**, pas un manquement : une PR en
//! brouillon ou déjà revue n'a rien à obtenir.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::lock::Locks;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Executable, Gate, OneShot, Stage};

use crate::pr_review::stages::Wiring;
use crate::pr_review::state::ReviewState;
use crate::pr_review::{skip_rules, stages};

/// Une revue entière : précontrôle, verrou, les deux passes, la publication.
pub struct ReviewRun {
    /// L'outillage que cette revue exige — rien de plus.
    pub pre: Gate<ReviewState>,
    /// Le tableau de PR.
    pub gh: Rc<dyn GitHub>,
    /// Ce qui tient les verrous.
    pub locks: Rc<dyn Locks>,
    /// Le dossier qui porte les verrous.
    pub review_dir: PathBuf,
    /// La PR à revoir, numéro ou URL.
    pub pr_ref: String,
    /// La branche que la PR doit cibler pour être revue.
    pub base: String,
    /// Revoit même si une règle dirait de sauter.
    pub force: bool,
    /// La table, déjà montée.
    pub stages: Vec<Stage<ReviewState>>,
}

#[async_trait(?Send)]
impl OneShot<ReviewState> for ReviewRun {
    fn pre(&self) -> Option<&Gate<ReviewState>> {
        Some(&self.pre)
    }

    async fn precheck(&self, ctx: &mut Context<ReviewState>) -> Outcome<Option<String>> {
        let pr = self.gh.pr(&self.pr_ref).await?;
        let comments = if self.force {
            String::new()
        } else {
            self.gh.pr_comments(&pr.num).await?
        };
        if let Some(skip) = skip_rules::skip_reason(self.force, &self.base, &pr, &comments) {
            return Ok(Some(format!("skip — {skip}")));
        }
        ctx.traces.say(&format!(
            "reviewing #{}  {} -> {}  ({})",
            pr.num, pr.head, pr.base, pr.title
        ));
        ctx.state.pr = Some(pr);
        Ok(None)
    }

    fn lock(&self) -> (&std::path::Path, &str) {
        (&self.review_dir, &self.pr_ref)
    }

    fn locks(&self) -> &dyn Locks {
        self.locks.as_ref()
    }

    fn held(&self, ctx: &Context<ReviewState>) -> String {
        let num = ctx
            .state
            .pr
            .as_ref()
            .map_or(self.pr_ref.as_str(), |pr| &pr.num);
        format!("skip — a review of PR #{num} is already running")
    }

    fn stages(&self) -> &[Stage<ReviewState>] {
        &self.stages
    }

    fn tolerate(&self, stage: &str, failed: &Halt) -> Option<String> {
        // La passe 1 peut ne rien rendre sans que les notes perdent leur
        // valeur. Un quota épuisé est l'exception : la passe 2 dépenserait la
        // même fenêtre et reviendrait pareil.
        if stage != "inline" || matches!(failed, Halt::Quota(_)) {
            return None;
        }
        Some("inline pass produced no review — continuing without it".to_string())
    }

    fn summary(&self, ctx: &Context<ReviewState>) -> String {
        if ctx.settings.dry_run {
            return "dry run — nothing posted".to_string();
        }
        format!("reviewed #{}", ctx.state.pr().num)
    }
}

#[async_trait(?Send)]
impl Executable<ReviewState> for ReviewRun {
    fn pre(&self) -> Option<&Gate<ReviewState>> {
        OneShot::pre(self)
    }

    async fn perform(&self, ctx: &mut Context<ReviewState>) -> Outcome<Verdict> {
        OneShot::execute(self, ctx).await
    }
}

/// Monte une revue entière, à partir de son câblage.
#[must_use]
pub fn build(
    wiring: &Wiring,
    locks: Rc<dyn Locks>,
    pr_ref: String,
    base: String,
    force: bool,
    pre: Gate<ReviewState>,
) -> ReviewRun {
    ReviewRun {
        pre,
        gh: Rc::clone(&wiring.gh),
        locks,
        review_dir: wiring.review_dir.clone(),
        pr_ref,
        base,
        force,
        stages: stages::table(wiring),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use crate::pr_review::stages::fake;
    use harness_core::adapters::store::lock::DirLocks;
    use harness_core::domain::Pr;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx() -> Context<ReviewState> {
        Context::new(
            Settings {
                dry_run: true,
                stages: String::new(),
            },
            ReviewState::default(),
            Logbook::null(),
        )
    }

    fn pr(num: &str, base: &str, draft: bool) -> Pr {
        Pr {
            num: num.to_string(),
            base: base.to_string(),
            head: "feat/x".to_string(),
            title: "un lot".to_string(),
            url: format!("https://github.com/o/r/pull/{num}"),
            state: "OPEN".to_string(),
            draft,
        }
    }

    fn run(gh: &Rc<FakeGitHub>, pr_ref: &str) -> ReviewRun {
        let wiring = fake::with(Rc::clone(gh));
        build(
            &wiring,
            Rc::new(DirLocks),
            pr_ref.to_string(),
            "main_agent".to_string(),
            false,
            Gate::empty("outillage"),
        )
    }

    #[tokio::test]
    async fn a_draft_pr_is_skipped_as_a_success_not_an_error() {
        let gh = Rc::new(FakeGitHub {
            prs: vec![("32".to_string(), pr("32", "main_agent", true))],
            ..FakeGitHub::default()
        });
        let built = run(&gh, "32");
        let mut context = ctx();
        built
            .execute(&mut context)
            .await
            .expect("succès, pas erreur");
        assert!(context.state.pr.is_none(), "le précontrôle n'a rien posé");
    }

    #[tokio::test]
    async fn an_unreadable_pr_is_a_real_failure() {
        let gh = Rc::new(FakeGitHub::default());
        let built = run(&gh, "999");
        let mut context = ctx();
        let err = built.execute(&mut context).await.expect_err("doit échouer");
        assert!(matches!(err, Halt::Failed(_)));
    }
}
