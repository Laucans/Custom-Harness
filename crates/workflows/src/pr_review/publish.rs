//! Le commentaire de synthèse : monté, gardé sur disque, puis posté.
//!
//! Écrit avant d'être posté : si `gh` échoue, le texte existe encore et le
//! message d'erreur peut dire où — deux passes payantes ne se perdent pas
//! parce qu'un appel réseau a raté.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::review_ledger::ReviewLedger;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};

use crate::pr_review::notes;
use crate::pr_review::state::ReviewState;

/// L'étape de publication : monte le commentaire, le garde, le poste.
///
/// Une étape locale qui ne paie rien. Le corps est ce que la passe « brief »
/// a rendu — lu dans `ctx.results`, sous son nom.
pub struct Publish {
    /// De quoi poster le commentaire.
    pub gh: Rc<dyn GitHub>,
    /// Où le commentaire est gardé, et où le registre vit.
    pub review_dir: PathBuf,
    /// `--no-inline`, pour le pied de page.
    pub no_inline: bool,
    /// Le niveau de `/code-review`, pour le pied de page.
    pub level: String,
    /// Le modèle de la passe « inline », pour le pied de page.
    pub inline_model: String,
    /// Le modèle de la passe « brief », pour le pied de page.
    pub brief_model: String,
    /// L'instant à afficher dans l'en-tête du commentaire.
    ///
    /// Un pointeur de fonction et non un appel direct à une horloge : ni
    /// `harness-core` ni les workflows ne portent de dépendance au temps,
    /// comme `adapters::store::spending` le documente déjà — c'est le
    /// lanceur qui sait quelle heure il est, et qui le fournit ici.
    pub now: fn() -> String,
}

impl Publish {
    fn passes_line(&self) -> String {
        if self.no_inline {
            format!(
                "notes `{}` (passe ligne à ligne désactivée)",
                self.brief_model
            )
        } else {
            format!(
                "findings `{}`/niveau `{}`, notes `{}`",
                self.inline_model, self.level, self.brief_model
            )
        }
    }
}

#[async_trait(?Send)]
impl Action<ReviewState> for Publish {
    async fn run(&self, ctx: &mut Context<ReviewState>) -> Outcome<Verdict> {
        let pr = ctx.state.pr().clone();
        let body = ctx
            .results
            .get("brief")
            .map(|reply| reply.text.clone())
            .unwrap_or_default();

        let ledger = ReviewLedger::new(&self.review_dir.join("costs.tsv"));
        let cost = ledger.cost_of(&pr.num)?;
        let path = self.review_dir.join(format!("{}-comment.md", pr.num));
        let stamp = (self.now)();
        let footer = notes::footer(
            &self.passes_line(),
            &if cost.is_empty() {
                String::new()
            } else {
                format!(", coût ${cost}")
            },
        );
        let full = notes::comment(&stamp, &body, &footer);
        // Écrit avant d'être posté : si `gh` échoue, le texte existe encore et
        // le message d'erreur peut dire où.
        std::fs::create_dir_all(&self.review_dir).map_err(|e| {
            Halt::Failed(format!(
                "impossible de créer {} : {e}",
                self.review_dir.display()
            ))
        })?;
        std::fs::write(&path, &full)
            .map_err(|e| Halt::Failed(format!("impossible d'écrire {} : {e}", path.display())))?;

        if let Err(why) = self.gh.post_pr_comment(&pr.num, &path).await {
            return Err(Halt::Failed(format!(
                "gh could not post the comment on PR #{} ({}) — the text is \
                 kept at {}; post it by hand: gh pr comment {} --body-file {}",
                pr.num,
                why.reason(),
                path.display(),
                pr.num,
                path.display()
            )));
        }
        ctx.traces.say(&format!(
            "posted — {}  (this review cost ${})",
            pr.url,
            if cost.is_empty() { "?" } else { &cost }
        ));
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::adapters::agent::Reply;
    use harness_core::domain::{Pr, Spend};
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("harness-pr-publish-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn ctx_with_brief(text: &str) -> Context<ReviewState> {
        let mut context = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            ReviewState {
                pr: Some(Pr {
                    num: "32".to_string(),
                    url: "https://github.com/o/r/pull/32".to_string(),
                    ..Pr::default()
                }),
            },
            Logbook::null(),
        );
        context.results.insert(
            "brief".to_string(),
            Reply {
                text: text.to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        context
    }

    #[tokio::test]
    async fn the_comment_is_written_to_disk_then_posted() {
        let review_dir = dir("writes-then-posts");
        let gh = Rc::new(FakeGitHub::default());
        let publish = Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            review_dir: review_dir.clone(),
            no_inline: false,
            level: "medium".to_string(),
            inline_model: "sonnet".to_string(),
            brief_model: "sonnet".to_string(),
            now: || "2026-10-02 16:00".to_string(),
        };
        let mut context = ctx_with_brief("les notes de la passe 2");
        publish.run(&mut context).await.expect("posté");

        let path = review_dir.join("32-comment.md");
        let text = std::fs::read_to_string(&path).expect("écrit sur disque");
        assert!(text.contains("les notes de la passe 2"));
        assert!(text.starts_with(notes::MARKER));

        assert_eq!(
            gh.writes(),
            vec![Wrote::PrComment(
                "32".to_string(),
                path.display().to_string()
            )]
        );
        let _ = std::fs::remove_dir_all(&review_dir);
    }

    #[tokio::test]
    async fn a_failed_post_names_where_the_text_is_and_how_to_post_it_by_hand() {
        let review_dir = dir("failed-post");
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Failed("réseau coupé".to_string())),
            ..FakeGitHub::default()
        });
        let publish = Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            review_dir: review_dir.clone(),
            no_inline: false,
            level: "medium".to_string(),
            inline_model: "sonnet".to_string(),
            brief_model: "sonnet".to_string(),
            now: || "2026-10-02 16:00".to_string(),
        };
        let mut context = ctx_with_brief("les notes");
        let err = publish.run(&mut context).await.expect_err("doit échouer");
        assert!(err.reason().contains("gh pr comment 32 --body-file"));
        // Le texte reste malgré l'échec de la publication.
        assert!(review_dir.join("32-comment.md").exists());
        let _ = std::fs::remove_dir_all(&review_dir);
    }

    #[tokio::test]
    async fn the_footer_names_no_inline_when_the_pass_was_disabled() {
        let review_dir = dir("no-inline-footer");
        let gh = Rc::new(FakeGitHub::default());
        let publish = Publish {
            gh,
            review_dir: review_dir.clone(),
            no_inline: true,
            level: "medium".to_string(),
            inline_model: "sonnet".to_string(),
            brief_model: "sonnet".to_string(),
            now: || "2026-10-02 16:00".to_string(),
        };
        let mut context = ctx_with_brief("les notes");
        publish.run(&mut context).await.expect("posté");
        let text = std::fs::read_to_string(review_dir.join("32-comment.md")).expect("écrit");
        assert!(text.contains("passe ligne à ligne désactivée"));
        let _ = std::fs::remove_dir_all(&review_dir);
    }
}
