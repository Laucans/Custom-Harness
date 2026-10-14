//! The diagnosis: one paid stage, and its text.
//!
//! **The only design surface of the workflow.** The prompt lives here, with
//! the table that sends it; the action receives it as a field.

use std::rc::Rc;

use harness_core::execution::{Stage, StageBody};

use crate::data_diagnosis::action::ask::AskDiagnosis;
use crate::data_diagnosis::config::Config;
use crate::data_diagnosis::data::state::DiagnosisState;
use crate::data_diagnosis::ports::Ports;

/// The name of the stage. Written here, nowhere else.
pub const DIAGNOSE: &str = "diagnose";

/// What the session is asked.
pub const PROMPT: &str = "You are diagnosing why the two sides of a product's data model disagree,
in the repository cloned in this folder. Nothing here is yours to change.

The two sides, for commit {sha} of the branch {branch}, are archived in
this folder (the archive branch `harness/data-model` is checked out):
  - `{dir}/schema.sql` — the database's side: what `scripts/dump-schema.sh`
    printed, the schema the migrations really build (`pg_dump --schema-only`);
  - `{dir}/model.json` — the agents' side: `data/model.json` at that commit,
    what every table and field is for (contract M,
    `contracts/m-data-model.schema.json`).

A static comparison of the two found:
{divergences}

The code at that commit is not checked out: read it through git —
  git show {sha}:<path>
  git log --oneline -20 {sha} -- <path>
  git ls-tree -r --name-only {sha}
The migrations live under a `migrations/` folder; `data/model.json` is the
agents' side as committed. Rule 14 of the repository's `CLAUDE.md` asks every
task that changes the structure to update `data/model.json` in the same pull
request.

For each divergence, decide which side is wrong — `database` (a migration is
missing or wrong), `model` (data/model.json was not updated, or says
something false), `both`, or `unclear` — and why, from the history: which
commit changed one side and not the other. Then the fix: the migration to
write, or what to change in data/model.json, concretely.

Do not edit any file, do not commit, do not post anything.

Answer with ONE JSON object and nothing else, in a ```json fence:
{\"summary\": \"<two or three sentences: the state of affairs, for a human who has not looked>\",
 \"findings\": [{\"divergence\": \"<as listed above>\", \"wrong\": \"database|model|both|unclear\",
                \"why\": \"<one or two sentences>\", \"fix\": \"<the change to make>\"}],
 \"actions\": [\"<what a human can decide, imperative, one sentence>\"]}";

/// The diagnosis, one paid stage.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<DiagnosisState>> {
    vec![Stage {
        name: DIAGNOSE.to_string(),
        pre: None,
        post: None,
        body: StageBody::Session {
            spec: config.spec.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskDiagnosis {
                stage: DIAGNOSE.to_string(),
                template: PROMPT.to_string(),
                spending: Rc::clone(&ports.spending),
            })],
        },
    }]
}
