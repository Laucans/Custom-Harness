//! What a product's units plug into the engine: one [`Plugin`] per unit,
//! gathered into [`Plugins`].
//!
//! A `DataCapability` crate brings a [`Prepare`]: the code that turns a
//! submission into the payload and the row it writes, or refuses it. An
//! aggregate crate brings a [`Judge`] (its invariants written as code) and
//! a [`Project`] (what follows an applied write). A Capability brings an
//! [`Answer`]: its query and how its rows become the answer. What units
//! read but do not own — a model client, a price list — sits in [`Shared`],
//! keyed by type; a plugin declares what it reads with [`Plugin::needs`],
//! and the engine refuses to start when a need has nobody to fill it.
//!
//! Over PostgreSQL (the `postgres` feature), an aggregate's projection may
//! write the product's tables inside the very transaction that applies the
//! write: a [`ProjectPg`], which the store runs, and whose refusal rejects
//! the command as an invariant would.

use std::any::{Any, TypeId, type_name};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

#[cfg(feature = "postgres")]
use crate::Error;
use crate::command::Entry;
use crate::guard::Caller;
use crate::manifest::DataCapability;
use crate::store::{Ids, Write};

/// What units share without owning: one value per type.
#[derive(Default, Clone)]
pub struct Shared {
    values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Shared {
    /// Keeps `value`, in place of any earlier value of the same type.
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: Arc<T>) {
        self.values.insert(TypeId::of::<T>(), value);
    }

    /// The value of type `T`, when one was given.
    #[must_use]
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.values
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|value| value.downcast::<T>().ok())
    }

    /// True when a value of this type was given.
    pub(crate) fn holds(&self, id: TypeId) -> bool {
        self.values.contains_key(&id)
    }

    /// Takes every value of `other`, which wins on a type both hold.
    pub(crate) fn absorb(&mut self, other: Self) {
        self.values.extend(other.values);
    }
}

/// What a [`Prepare`] sees beside the submission.
pub struct Context<'a> {
    /// The `DataCapability` submitted.
    pub manifest: &'a DataCapability,
    /// Who submits.
    pub caller: &'a Caller,
    /// The target row as the commands still pending leave it: empty when
    /// the submission names no row yet.
    pub base: &'a BTreeMap<String, Value>,
    /// Now, in seconds since the epoch.
    pub now: u64,
    /// Where a fresh row id comes from.
    pub ids: &'a dyn Ids,
    /// What the units share.
    pub shared: &'a Shared,
}

/// What a [`Prepare`] turns a submission into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prepared {
    /// The payload the command writes, in place of the one submitted.
    pub payload: BTreeMap<String, Value>,
    /// The row's id; when absent, the payload names it or a fresh one is
    /// taken.
    pub target_id: Option<String>,
    /// What the author should know, beside the engine's own warnings.
    pub warnings: Vec<String>,
}

/// Why a [`Prepare`] refuses: what the server answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalKind {
    /// Something the submission names does not exist (404).
    NotFound,
    /// The submission is well formed but cannot be done (422).
    Unprocessable,
}

/// A submission a [`Prepare`] refuses, before anything is queued.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Refusal {
    /// What kind of refusal.
    pub kind: RefusalKind,
    /// Why, for the author.
    pub message: String,
}

impl Refusal {
    /// Something the submission names does not exist.
    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: RefusalKind::NotFound,
            message: message.into(),
        }
    }

    /// The submission cannot be done.
    #[must_use]
    pub fn unprocessable(message: impl Into<String>) -> Self {
        Self {
            kind: RefusalKind::Unprocessable,
            message: message.into(),
        }
    }
}

/// A `DataCapability`'s code: the submission turned into what it writes.
#[async_trait]
pub trait Prepare: Send + Sync {
    /// The payload and the row the submission writes, or why it may not.
    ///
    /// # Errors
    ///
    /// A [`Refusal`]: nothing is queued.
    async fn prepare(
        &self,
        context: &Context<'_>,
        payload: &BTreeMap<String, Value>,
        target_id: Option<&str>,
    ) -> Result<Prepared, Refusal>;
}

/// An aggregate's invariants written as code, beside its declared rules.
pub trait Judge: Send + Sync {
    /// The ids of the aggregate's invariants that `state` — the row after
    /// the command — violates.
    fn violated(&self, state: &BTreeMap<String, Value>, entry: &Entry) -> Vec<String>;
}

/// What follows an applied write of an aggregate: a read table, a cache.
pub trait Project: Send + Sync {
    /// Sees one write the store applied.
    fn observe(&self, write: &Write);
}

/// Why a [`ProjectPg`] did not project a write.
#[cfg(feature = "postgres")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    /// The write breaks what the product's tables hold: the transaction
    /// rolls back, the command is rejected and its author told why.
    #[error("{0}")]
    Refused(String),
    /// The projection could not run: the store failed, the step fails.
    #[error("the projection failed: {0}")]
    Failed(String),
}

#[cfg(feature = "postgres")]
impl From<tokio_postgres::Error> for ProjectionError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Failed(crate::postgres::described(&error))
    }
}

/// A refused projection is the product's code refusing the command, as a
/// `Prepare` would (`Unprocessable`); a failed one is the store's error.
#[cfg(feature = "postgres")]
impl From<ProjectionError> for Error {
    fn from(error: ProjectionError) -> Self {
        match error {
            ProjectionError::Refused(message) => Self::Refused(Refusal::unprocessable(message)),
            ProjectionError::Failed(message) => Self::Store(message),
        }
    }
}

/// What follows an applied write of an aggregate, in the product's
/// PostgreSQL tables, inside the transaction that applies it.
#[cfg(feature = "postgres")]
#[async_trait]
pub trait ProjectPg: Send + Sync {
    /// Writes what follows `write`, after the engine's own row and before
    /// the commit: the write and its projection commit or roll back
    /// together.
    ///
    /// # Errors
    ///
    /// [`ProjectionError::Refused`] rejects the command,
    /// [`ProjectionError::Failed`] fails the step; both roll back.
    async fn project(
        &self,
        write: &Write,
        tx: &tokio_postgres::Transaction<'_>,
    ) -> Result<(), ProjectionError>;

    /// Makes ready what the projection needs, once, when the store starts.
    ///
    /// # Errors
    ///
    /// A [`ProjectionError`]: the store does not start.
    async fn warm(&self, _client: &tokio_postgres::Client) -> Result<(), ProjectionError> {
        Ok(())
    }
}

/// A Capability's read: its query, and how its rows become the answer.
#[derive(Debug, Clone)]
pub struct Answer {
    /// The Capability's id.
    pub capability: String,
    /// The query its rows come from.
    pub sql: String,
    /// The request's variables bound to the query, in order.
    pub variables: Vec<String>,
    /// The answer, from the rows (a JSON array) and the request's variables.
    pub answer: fn(&Value, &BTreeMap<String, Value>) -> Value,
}

/// A type a plugin reads from [`Shared`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Need {
    pub(crate) id: TypeId,
    pub(crate) name: &'static str,
}

/// What one unit plugs in.
#[derive(Default)]
pub struct Plugin {
    pub(crate) prepares: Vec<(String, Arc<dyn Prepare>)>,
    pub(crate) judges: Vec<(String, Arc<dyn Judge>)>,
    pub(crate) projects: Vec<(String, Arc<dyn Project>)>,
    #[cfg(feature = "postgres")]
    pub(crate) project_pgs: Vec<(String, Arc<dyn ProjectPg>)>,
    pub(crate) answers: Vec<Answer>,
    pub(crate) shared: Shared,
    pub(crate) needs: Vec<Need>,
}

impl Plugin {
    /// A plugin that plugs nothing in yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The code of the `DataCapability` named `data_capability`.
    #[must_use]
    pub fn prepare(mut self, data_capability: &str, prepare: impl Prepare + 'static) -> Self {
        self.prepares
            .push((data_capability.to_string(), Arc::new(prepare)));
        self
    }

    /// Invariants of the aggregate named `aggregate`, as code.
    #[must_use]
    pub fn judge(mut self, aggregate: &str, judge: impl Judge + 'static) -> Self {
        self.judges.push((aggregate.to_string(), Arc::new(judge)));
        self
    }

    /// What follows a write of the aggregate named `aggregate`.
    #[must_use]
    pub fn project(mut self, aggregate: &str, project: impl Project + 'static) -> Self {
        self.projects
            .push((aggregate.to_string(), Arc::new(project)));
        self
    }

    /// What follows a write of the aggregate named `aggregate`, in the
    /// transaction that applies it: the PostgreSQL store runs it.
    #[cfg(feature = "postgres")]
    #[must_use]
    pub fn project_pg(mut self, aggregate: &str, project: impl ProjectPg + 'static) -> Self {
        self.project_pgs
            .push((aggregate.to_string(), Arc::new(project)));
        self
    }

    /// A Capability's read.
    #[must_use]
    pub fn answer(mut self, answer: Answer) -> Self {
        self.answers.push(answer);
        self
    }

    /// A value the plugin brings for every unit to read.
    #[must_use]
    pub fn share<T: Send + Sync + 'static>(mut self, value: Arc<T>) -> Self {
        self.shared.insert(value);
        self
    }

    /// Declares that the plugin reads a `T` from [`Shared`]: the engine
    /// refuses to load without one.
    #[must_use]
    pub fn needs<T: Send + Sync + 'static>(mut self) -> Self {
        self.needs.push(Need {
            id: TypeId::of::<T>(),
            name: type_name::<T>(),
        });
        self
    }

    /// Takes everything `other` plugs in.
    fn merge(&mut self, other: Self) {
        self.prepares.extend(other.prepares);
        self.judges.extend(other.judges);
        self.projects.extend(other.projects);
        #[cfg(feature = "postgres")]
        self.project_pgs.extend(other.project_pgs);
        self.answers.extend(other.answers);
        self.shared.absorb(other.shared);
        self.needs.extend(other.needs);
    }
}

/// Every unit's plugin, together.
#[derive(Default)]
pub struct Plugins(pub(crate) Plugin);

impl FromIterator<Plugin> for Plugins {
    fn from_iter<I: IntoIterator<Item = Plugin>>(plugins: I) -> Self {
        let mut all = Plugin::new();
        for plugin in plugins {
            all.merge(plugin);
        }
        Self(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Model(&'static str);
    struct Prices;

    struct Nothing;

    impl Judge for Nothing {
        fn violated(&self, _state: &BTreeMap<String, Value>, _entry: &Entry) -> Vec<String> {
            Vec::new()
        }
    }

    impl Project for Nothing {
        fn observe(&self, _write: &Write) {}
    }

    #[test]
    fn shared_answers_by_type_and_the_last_value_of_a_type_wins() {
        let mut shared = Shared::default();
        assert!(shared.get::<Model>().is_none(), "nothing given");
        shared.insert(Arc::new(Model("first")));
        shared.insert(Arc::new(Model("second")));
        assert_eq!(shared.get::<Model>().map(|m| m.0), Some("second"));
        assert!(shared.get::<Prices>().is_none(), "another type");
        assert!(shared.holds(TypeId::of::<Model>()));
        assert!(!shared.holds(TypeId::of::<Prices>()));
        let mut other = Shared::default();
        other.insert(Arc::new(Prices));
        other.insert(Arc::new(Model("third")));
        shared.absorb(other);
        assert!(shared.get::<Prices>().is_some());
        assert_eq!(shared.get::<Model>().map(|m| m.0), Some("third"));
    }

    #[test]
    fn a_refusal_carries_its_kind_and_reads_as_its_message() {
        let missing = Refusal::not_found("no account 9");
        assert_eq!(missing.kind, RefusalKind::NotFound);
        assert_eq!(missing.to_string(), "no account 9");
        assert_eq!(
            Refusal::unprocessable("closed").kind,
            RefusalKind::Unprocessable
        );
    }

    #[cfg(feature = "postgres")]
    #[async_trait]
    impl ProjectPg for Nothing {
        async fn project(
            &self,
            _write: &Write,
            _tx: &tokio_postgres::Transaction<'_>,
        ) -> Result<(), ProjectionError> {
            Ok(())
        }
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn a_refused_projection_is_a_refusal_and_a_failed_one_the_store_s_error() {
        let refused = Error::from(ProjectionError::Refused("the ledger is closed".to_string()));
        assert_eq!(
            refused,
            Error::Refused(Refusal::unprocessable("the ledger is closed"))
        );
        assert_eq!(
            Error::from(ProjectionError::Failed("disk full".to_string())),
            Error::Store("disk full".to_string())
        );
        assert_eq!(
            ProjectionError::Refused("closed".to_string()).to_string(),
            "closed"
        );
        let unparsable = "host=a port=nope"
            .parse::<tokio_postgres::Config>()
            .expect_err("not a port");
        let failed = ProjectionError::from(unparsable);
        assert!(
            matches!(&failed, ProjectionError::Failed(why) if why.contains("invalid connection string")),
            "{failed:?}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn plugins_gather_the_projections_the_store_runs() {
        let plugins: Plugins = [
            Plugin::new().project_pg("Account", Nothing),
            Plugin::new()
                .project_pg("Board", Nothing)
                .project_pg("Account", Nothing),
        ]
        .into_iter()
        .collect();
        let aggregates: Vec<&str> = plugins
            .0
            .project_pgs
            .iter()
            .map(|(aggregate, _)| aggregate.as_str())
            .collect();
        assert_eq!(aggregates, ["Account", "Board", "Account"]);
        assert!(plugins.0.projects.is_empty(), "not an in-memory Project");
    }

    #[test]
    fn plugins_gather_what_every_plugin_brings() {
        let answer = Answer {
            capability: "credit.risk".to_string(),
            sql: "select 1".to_string(),
            variables: Vec::new(),
            answer: |rows, _| rows.clone(),
        };
        let plugins: Plugins = [
            Plugin::new()
                .judge("Account", Nothing)
                .needs::<Model>()
                .share(Arc::new(Prices)),
            Plugin::new()
                .project("Account", Nothing)
                .answer(answer)
                .share(Arc::new(Model("m"))),
        ]
        .into_iter()
        .collect();
        let all = &plugins.0;
        assert_eq!(all.judges.len(), 1);
        assert_eq!(all.projects.len(), 1);
        assert_eq!(all.answers.len(), 1);
        assert!(all.shared.get::<Prices>().is_some());
        assert!(all.shared.get::<Model>().is_some());
        assert_eq!(all.needs.len(), 1);
        assert_eq!(all.needs[0].id, TypeId::of::<Model>());
        assert!(
            all.needs[0].name.ends_with("Model"),
            "{}",
            all.needs[0].name
        );
    }
}
