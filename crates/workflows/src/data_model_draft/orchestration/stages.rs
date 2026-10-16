//! The draft: prepare, draft, publish — and the session's text.
//!
//! **The only design surface of the workflow.**

use std::rc::Rc;

use harness_core::execution::{Stage, StageBody};

use crate::data_model_draft::action::ask::AskDraft;
use crate::data_model_draft::action::prepare::Prepare;
use crate::data_model_draft::action::publish::Publish;
use crate::data_model_draft::config::Config;
use crate::data_model_draft::data::state::DraftState;
use crate::data_model_draft::ports::Ports;

/// The clone, the dump, the draft branch.
pub const PREPARE: &str = "prepare";
/// The session that writes the model.
pub const DRAFT: &str = "draft";
/// The commit and the pull request.
pub const PUBLISH: &str = "publish";

/// What the session is asked.
pub const PROMPT: &str =
    "Write `data/model.json` for this repository, checked out here at commit {sha} of the
branch {base}, on a branch of its own. It does not exist yet.

`data/model.json` is the agents' side of the product's data model: what every
table and field is *for*. Its format is contract M —
`contracts/m-data-model.schema.json` (when this repository does not carry it
yet: an object `{\"version\": 1, \"tables\": [...], \"links\": [...]}`; each table has
`name`, `purpose`, `business` (rules, a sentence each) and `fields`; each field
has `name`, `type`, `nullable`, `purpose`, optionally `technical`, `business`
and `references` (`<table>.<field>`, for a foreign key); `links` are relations
outside any foreign key, each with `from`, `to`, `purpose`).

The database's side is the schema the migrations build, dumped for you at:
  {schema}
Describe **every** table and field of that dump, with the dump's names, its
types (as Postgres writes them) and its nullability, and a `references` for
each foreign key — a static check compares the two sides afterwards. Then
read the code that uses them — migrations, queries, the DataGuard, the
capabilities, the UI — and write, for each table and field, its purpose in a
sentence and the business rules the code enforces or assumes. Write what the
code shows; when a rule is only implied, say so (\"assumed: …\"); never invent
one.

Write that one file and nothing else: do not edit any other file, do not
commit, do not push — the harness checks the file, commits it and opens the
pull request. End with one line: `DRAFTED: <number of tables> tables`.";

/// The draft's three stages.
#[must_use]
pub fn table(ports: &Ports, config: &Config) -> Vec<Stage<DraftState>> {
    vec![
        Stage {
            name: PREPARE.to_string(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(Prepare::new(ports, config))],
            },
        },
        Stage {
            name: DRAFT.to_string(),
            pre: None,
            post: None,
            body: StageBody::Session {
                turn_cap: None,
                spec: config.spec.clone(),
                sessions: Rc::clone(&ports.sessions),
                actions: vec![Box::new(AskDraft {
                    stage: DRAFT.to_string(),
                    template: PROMPT.to_string(),
                    base: config.base.clone(),
                    spending: Rc::clone(&ports.spending),
                })],
            },
        },
        Stage {
            name: PUBLISH.to_string(),
            pre: None,
            post: None,
            body: StageBody::Local {
                actions: vec![Box::new(Publish {
                    gh: Rc::clone(&ports.gh),
                    repos: Rc::clone(&ports.repos),
                    disk: Rc::clone(&ports.disk),
                    base: config.base.clone(),
                })],
            },
        },
    ]
}
