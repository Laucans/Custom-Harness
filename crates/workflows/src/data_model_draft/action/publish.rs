//! The drafted file checked, committed alone, pushed, and proposed as a pull
//! request on the base branch.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Action, Context};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repos;
use harness_core::ports::shell::github::GitHub;

use super::said;
use crate::data_model_draft::data::state::DraftState;
use crate::data_snapshot::data::model::{self, Declared, MODEL_PATH};

/// Publishes the draft.
pub struct Publish {
    /// Where the pull request is opened.
    pub gh: Rc<dyn GitHub>,
    /// `git`, on the draft clone.
    pub repos: Rc<dyn Repos>,
    /// Reads the drafted file.
    pub disk: Rc<dyn Disk>,
    /// The base branch.
    pub base: String,
}

/// The pull request's body: what it is, and where the draft still disagrees.
#[must_use]
pub fn body(base: &str, sha: &str, tables: usize, divergences: &[String]) -> String {
    let short: String = sha.chars().take(7).collect();
    let mut text = format!(
        "A first `data/model.json` for `{base}`, drafted from its schema at `{short}` \
         (`scripts/dump-schema.sh`) and its code, as contract M asks: what each of its {tables} \
         table(s) and their fields are for, their business rules, the references between them.\n\n\
         Read the purposes and the business rules: they are a draft, written by an AI from the \
         code. The structure — types, nullability, references — is checked against the dump below.\n"
    );
    if divergences.is_empty() {
        text.push_str("\nThe draft agrees with the database's schema.\n");
    } else {
        text.push_str("\nWhere the draft still disagrees with the database's schema:\n\n");
        for d in divergences {
            text.push_str("- ");
            text.push_str(d);
            text.push('\n');
        }
    }
    text.push_str(
        "\n---\nOpened from the factory view's data pane (`harness data-model-draft`).\n",
    );
    text
}

#[async_trait(?Send)]
impl Action<DraftState> for Publish {
    async fn run(&self, ctx: &mut Context<DraftState>) -> Outcome<Verdict> {
        let clone = ctx.state.clone.clone();
        let repo = self.repos.at(&clone);
        // Only the model: a session that touched anything else is not published.
        let others: Vec<String> = repo
            .dirty_files()
            .await?
            .into_iter()
            .filter(|path| path != MODEL_PATH)
            .collect();
        if !others.is_empty() {
            return Err(Halt::Failed(format!(
                "the session changed more than data/model.json: {}",
                others.join(", ")
            )));
        }
        let text = self
            .disk
            .read_to_string(&clone.join(MODEL_PATH))
            .ok_or_else(|| Halt::Failed("the session wrote no data/model.json".to_string()))?;
        let declared: Declared = serde_json::from_str(&text)
            .map_err(|e| Halt::Failed(format!("data/model.json is not the model's JSON: {e}")))?;
        let merged = model::build("", &Ok(Some(ctx.state.schema.clone())), &Ok(Some(text)));
        ctx.state.divergences.clone_from(&merged.divergences);
        said("add", &repo.stage_all().await?)?;
        said(
            "commit",
            &repo
                .commit(&format!(
                    "docs(data): a first data/model.json for {}",
                    self.base
                ))
                .await?,
        )?;
        said("push", &repo.push(&ctx.state.branch).await?)?;
        let url = self
            .gh
            .create_pr(
                &ctx.state.branch,
                &self.base,
                &format!("A first data model for {}", self.base),
                &body(
                    &self.base,
                    &ctx.state.sha,
                    declared.tables.len(),
                    &merged.divergences,
                ),
            )
            .await?;
        ctx.traces.say(&format!("draft: opened {url}"));
        ctx.state.pr = Some(url);
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_says_what_to_read_and_where_it_disagrees() {
        let agree = body("main_agent", "abcdef0123", 5, &[]);
        assert!(agree.contains("`abcdef0`") && agree.contains("5 table(s)"));
        assert!(agree.contains("agrees with the database"));
        let not = body(
            "main",
            "a",
            1,
            &["table x is in the model, not in the database".into()],
        );
        assert!(not.contains("- table x is in the model, not in the database"));
    }
}
