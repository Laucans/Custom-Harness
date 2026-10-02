//! The summary comment: assembled, saved to disk, then posted.
//!
//! Written before posting: if `gh` fails, the text still exists and the error
//! message can say where — two paid passes don't vanish because a network call failed.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::review_ledger::ReviewLedger;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};

use crate::pr_review::data::notes;
use crate::pr_review::data::state::ReviewState;

/// The publishing step: assembles the comment, saves it, posts it.
///
/// A local stage that costs nothing. The body is what the "brief" pass
/// rendered — read from `ctx.results` under its name.
pub struct Publish {
    /// What posts the comment.
    pub gh: Rc<dyn GitHub>,
    /// The summary stage name, where its text is stored in `ctx.results`.
    /// Received from the table rather than hardcoded here: two literals would
    /// desynchronize by posting an empty comment.
    pub brief: String,
    /// Where the comment is saved and the ledger lives.
    pub review_dir: PathBuf,
    /// `--no-inline`, for the footer.
    pub no_inline: bool,
    /// The `/code-review` level, for the footer.
    pub level: String,
    /// The "inline" pass model, for the footer.
    pub inline_model: String,
    /// The "brief" pass model, for the footer.
    pub brief_model: String,
    /// The timestamp to show in the comment header.
    ///
    /// A function pointer, not a direct clock call: neither `harness-core`
    /// nor the workflows carry a time dependency, as
    /// `adapters::store::spending` already documents — it's the launcher
    /// that knows the time and supplies it here.
    pub now: fn() -> String,
}

impl Publish {
    fn passes_line(&self) -> String {
        if self.no_inline {
            format!("notes `{}` (line-by-line pass disabled)", self.brief_model)
        } else {
            format!(
                "findings `{}`/level `{}`, notes `{}`",
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
            .get(&self.brief)
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
                format!(", cost ${cost}")
            },
        );
        let full = notes::comment(&stamp, &body, &footer);
        // Written before posting: if `gh` fails, the text still exists and the
        // error message can say where.
        std::fs::create_dir_all(&self.review_dir).map_err(|e| {
            Halt::Failed(format!(
                "failed to create {}: {e}",
                self.review_dir.display()
            ))
        })?;
        std::fs::write(&path, &full)
            .map_err(|e| Halt::Failed(format!("failed to write {}: {e}", path.display())))?;

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
            brief: "brief".to_string(),
            review_dir: review_dir.clone(),
            no_inline: false,
            level: "medium".to_string(),
            inline_model: "sonnet".to_string(),
            brief_model: "sonnet".to_string(),
            now: || "2026-10-02 16:00".to_string(),
        };
        let mut context = ctx_with_brief("pass 2 notes");
        publish.run(&mut context).await.expect("posted");

        let path = review_dir.join("32-comment.md");
        let text = std::fs::read_to_string(&path).expect("written to disk");
        assert!(text.contains("pass 2 notes"));
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
            broken: Some(Halt::Failed("network down".to_string())),
            ..FakeGitHub::default()
        });
        let publish = Publish {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            brief: "brief".to_string(),
            review_dir: review_dir.clone(),
            no_inline: false,
            level: "medium".to_string(),
            inline_model: "sonnet".to_string(),
            brief_model: "sonnet".to_string(),
            now: || "2026-10-02 16:00".to_string(),
        };
        let mut context = ctx_with_brief("notes");
        let err = publish.run(&mut context).await.expect_err("must fail");
        assert!(err.reason().contains("gh pr comment 32 --body-file"));
        // Text remains despite posting failure.
        assert!(review_dir.join("32-comment.md").exists());
        let _ = std::fs::remove_dir_all(&review_dir);
    }

    #[tokio::test]
    async fn the_footer_names_no_inline_when_the_pass_was_disabled() {
        let review_dir = dir("no-inline-footer");
        let gh = Rc::new(FakeGitHub::default());
        let publish = Publish {
            gh,
            brief: "brief".to_string(),
            review_dir: review_dir.clone(),
            no_inline: true,
            level: "medium".to_string(),
            inline_model: "sonnet".to_string(),
            brief_model: "sonnet".to_string(),
            now: || "2026-10-02 16:00".to_string(),
        };
        let mut context = ctx_with_brief("notes");
        publish.run(&mut context).await.expect("posted");
        let text = std::fs::read_to_string(review_dir.join("32-comment.md")).expect("written");
        assert!(text.contains("line-by-line pass disabled"));
        let _ = std::fs::remove_dir_all(&review_dir);
    }
}
