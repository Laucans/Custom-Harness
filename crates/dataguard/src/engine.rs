//! The engine: the `DataGuard`, the `DataQueue`, the Resolver and the
//! single applier, driven together over a store, a notifier and a clock.
//!
//! One instance per server. A submission is judged (authorisation, the
//! effect it declares, the invariants on the projected state), enqueued in
//! its partition, and answered with where it stands; [`Engine::run_once`]
//! then drives every partition: the head that is ready is classified
//! again at execution, planned by the Resolver when it mutates existing
//! data, checked against the invariants on the real state — which alone
//! decides — and applied, taking the next `dataVersion`. The authors hear
//! each step (contract L).
//!
//! The product's units plug in ([`crate::plugins`]): a `DataCapability`'s
//! [`Prepare`] runs on a submission before its entry is built, an
//! aggregate's [`Judge`]s add to its declared rules, its [`Project`]s see
//! every write applied. A projection the store runs inside the apply
//! transaction (`ProjectPg`) may refuse the write: the store answers
//! [`Error::Refused`], and the head is rejected as an invariant rejects it.
//!
//! A `Prepare` may read any partition's row as the queue leaves it
//! ([`Context::row`]) and, over PostgreSQL, the product's tables through
//! read-only SQL; it may ask for follow-up submissions, which the engine
//! makes once the first command is queued ([`Prepared::then`]), and attach
//! what its author reads once the command is applied
//! ([`Prepared::attachment`]).
//!
//! The queue and the reviews live behind `std::sync::Mutex`: every lock is
//! taken and released between two awaits, never across one.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::aggregate::Aggregate;
use crate::api::{FollowUp, Submission, Submitted};
use crate::command::{BasedOn, CommandResult, Effect, Entry, Projection, State};
use crate::guard::{self, Caller, Classification, Invariants, Verdict};
use crate::manifest::{DataCapability, Idempotency};
use crate::memory::Counter;
use crate::messages::{Action, Kind, Message, Notifier};
use crate::plugins::{Context, Judge, Partitions, Plugins, Prepare, Prepared, Project, Shared};
use crate::queue::{Hold, Queue, Step};
use crate::reads::Rows;
use crate::reconcile::{Base, Candidate, NoProposer, Proposer, Review, Status};
use crate::registry::Registry;
use crate::resolver::{self, Decision, ImpactPlan, Situation};
use crate::store::{Clock, Ids, Store, Write};
use crate::{Error, Result};

/// How many heads one partition may go through in one run: a parked head
/// is set aside and the next looked at, up to this many times.
const HEADS_PER_RUN: usize = 64;

/// The aggregates' judges, each asked about its own aggregate only.
#[derive(Default)]
struct Judges(BTreeMap<String, Vec<Arc<dyn Judge>>>);

impl Invariants for Judges {
    fn violated(
        &self,
        aggregate: &str,
        state: &BTreeMap<String, Value>,
        entry: &Entry,
    ) -> Vec<String> {
        self.0
            .get(aggregate)
            .into_iter()
            .flatten()
            .flat_map(|judge| judge.violated(state, entry))
            .collect()
    }
}

/// The engine's rows lent to a `Prepare` ([`Context::row`]): the first
/// failure of the store is kept, and the engine answers it whatever the
/// `Prepare` made of it.
struct Lent<'e> {
    engine: &'e Engine,
    failed: Mutex<Option<Error>>,
}

#[async_trait::async_trait]
impl Partitions for Lent<'_> {
    async fn row(&self, partition: &str) -> Result<BTreeMap<String, Value>> {
        let row = self.engine.projected_row(partition).await;
        if let Err(failure) = &row {
            self.failed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_or_insert_with(|| failure.clone());
        }
        row
    }
}

/// The engine.
pub struct Engine {
    store: Arc<dyn Store>,
    notifier: Arc<dyn Notifier>,
    clock: Arc<dyn Clock>,
    row_ids: Arc<dyn Ids>,
    judges: Judges,
    prepares: Vec<(String, Arc<dyn Prepare>)>,
    projects: BTreeMap<String, Vec<Arc<dyn Project>>>,
    /// The aggregates whose projections the store runs (`ProjectPg`):
    /// the engine only checks that it registers them.
    stored_projections: Vec<String>,
    shared: Shared,
    needs: Vec<crate::plugins::Need>,
    /// Read-only SQL lent to the `Prepare`s, over PostgreSQL.
    rows: Option<Arc<dyn Rows>>,
    proposer: Arc<dyn Proposer>,
    aggregates: BTreeMap<String, Aggregate>,
    manifests: BTreeMap<String, DataCapability>,
    registry: Registry,
    queue: Mutex<Queue>,
    reviews: Mutex<BTreeMap<String, (Review, String)>>,
    plans: Mutex<BTreeMap<String, ImpactPlan>>,
    ids: AtomicU64,
}

/// What the `DataGuard` found on a submission, before it is enqueued.
struct Judged {
    warnings: Vec<String>,
    messages: Vec<Message>,
    needs_review: bool,
}

impl Engine {
    /// An engine over these ports, with no aggregate, no `DataCapability`
    /// and no registry yet.
    #[must_use]
    pub fn new(store: Arc<dyn Store>, notifier: Arc<dyn Notifier>, clock: Arc<dyn Clock>) -> Self {
        Self {
            store,
            notifier,
            clock,
            row_ids: Arc::new(Counter::default()),
            judges: Judges::default(),
            prepares: Vec::new(),
            projects: BTreeMap::new(),
            stored_projections: Vec::new(),
            shared: Shared::default(),
            needs: Vec::new(),
            rows: None,
            proposer: Arc::new(NoProposer),
            aggregates: BTreeMap::new(),
            manifests: BTreeMap::new(),
            registry: Registry::default(),
            queue: Mutex::new(Queue::default()),
            reviews: Mutex::new(BTreeMap::new()),
            plans: Mutex::new(BTreeMap::new()),
            ids: AtomicU64::new(1),
        }
    }

    /// Registers an aggregate (contract I).
    #[must_use]
    pub fn with_aggregate(mut self, aggregate: Aggregate) -> Self {
        self.aggregates
            .insert(aggregate.aggregate.clone(), aggregate);
        self
    }

    /// Registers a `DataCapability` (contract F).
    #[must_use]
    pub fn with_data_capability(mut self, manifest: DataCapability) -> Self {
        self.manifests
            .insert(manifest.data_capability.clone(), manifest);
        self
    }

    /// The registry of persisted queries (contract C).
    #[must_use]
    pub fn with_registry(mut self, registry: Registry) -> Self {
        self.registry = registry;
        self
    }

    /// The units' plugins: their `Prepare`s, `Judge`s and `Project`s, and
    /// what they share. Their `Answer`s are for [`crate::reads::Reads`],
    /// and their `ProjectPg`s for the PostgreSQL store, not the engine,
    /// which only checks their aggregates on [`Self::load`].
    #[must_use]
    pub fn with_plugins(mut self, plugins: Plugins) -> Self {
        let plugin = plugins.0;
        self.prepares.extend(plugin.prepares);
        for (aggregate, judge) in plugin.judges {
            self.judges.0.entry(aggregate).or_default().push(judge);
        }
        for (aggregate, project) in plugin.projects {
            self.projects.entry(aggregate).or_default().push(project);
        }
        #[cfg(feature = "postgres")]
        self.stored_projections.extend(
            plugin
                .project_pgs
                .into_iter()
                .map(|(aggregate, _)| aggregate),
        );
        self.shared.absorb(plugin.shared);
        self.needs.extend(plugin.needs);
        self
    }

    /// What the server shares with the units, beside what the plugins
    /// bring; it wins on a type both give.
    #[must_use]
    pub fn with_shared(mut self, shared: Shared) -> Self {
        self.shared.absorb(shared);
        self
    }

    /// Where a fresh row id comes from: a [`Counter`] by default.
    #[must_use]
    pub fn with_ids(mut self, ids: Arc<dyn Ids>) -> Self {
        self.row_ids = ids;
        self
    }

    /// Read-only SQL over the product's tables, lent to every `Prepare`
    /// ([`Context::rows`]); none by default, as over memory.
    #[must_use]
    pub fn with_rows(mut self, rows: Arc<dyn Rows>) -> Self {
        self.rows = Some(rows);
        self
    }

    /// The AI that proposes reconciliations.
    #[must_use]
    pub fn with_proposer(mut self, proposer: Arc<dyn Proposer>) -> Self {
        self.proposer = proposer;
        self
    }

    /// Loads what the store kept: the pending entries and the live holds,
    /// once the units it was given fit together ([`Self::fits`]).
    ///
    /// # Errors
    ///
    /// [`Error::Misconfigured`], or the store's.
    pub async fn load(&self) -> Result<()> {
        self.fits()?;
        let pending = self.store.pending().await?;
        let holds = self.store.holds().await?;
        *self.lock_queue() = Queue::load(pending, holds);
        Ok(())
    }

    /// Whether the units fit together: every invariant a `DataCapability`
    /// lists is one its target aggregate declares — a manifest naming
    /// another is a typo the `DataGuard` would never judge — and every
    /// plugin names a unit the engine knows and finds what it needs.
    fn fits(&self) -> Result<()> {
        self.plugged()?;
        for manifest in self.manifests.values() {
            let aggregate = self.aggregates.get(&manifest.target.aggregate);
            let unknown: Vec<&str> = manifest
                .invariants
                .iter()
                .filter(|id| aggregate.is_none_or(|a| a.invariant(id).is_none()))
                .map(String::as_str)
                .collect();
            if !unknown.is_empty() {
                return Err(Error::Misconfigured(format!(
                    "{} lists the invariant(s) {} that {} does not declare",
                    manifest.versioned(),
                    unknown.join(", "),
                    manifest.target.aggregate
                )));
            }
        }
        Ok(())
    }

    /// Whether every plugin fits: a `Prepare` for a `DataCapability` the
    /// engine registers, once; a `Judge`, a `Project` or a `ProjectPg` for
    /// an aggregate it registers; every need found in [`Shared`].
    fn plugged(&self) -> Result<()> {
        for (index, (name, _)) in self.prepares.iter().enumerate() {
            if !self.manifests.contains_key(name) {
                return Err(Error::Misconfigured(format!(
                    "a Prepare for {name}, a DataCapability the engine does not register"
                )));
            }
            if self.prepares[..index]
                .iter()
                .any(|(other, _)| other == name)
            {
                return Err(Error::Misconfigured(format!("two Prepares for {name}")));
            }
        }
        let aggregates = self
            .judges
            .0
            .keys()
            .map(|a| ("Judge", a))
            .chain(self.projects.keys().map(|a| ("Project", a)))
            .chain(self.stored_projections.iter().map(|a| ("ProjectPg", a)));
        for (kind, aggregate) in aggregates {
            if !self.aggregates.contains_key(aggregate) {
                return Err(Error::Misconfigured(format!(
                    "a {kind} for {aggregate}, an aggregate the engine does not register"
                )));
            }
        }
        let missing: Vec<&str> = self
            .needs
            .iter()
            .filter(|need| !self.shared.holds(need.id))
            .map(|need| need.name)
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(Error::Misconfigured(format!(
                "a plugin needs {}, which nothing shares",
                missing.join(", ")
            )))
        }
    }

    /// The fields of `partition`'s aggregate that merge instead of asking.
    fn never_stale(&self, partition: &str) -> Vec<String> {
        partition
            .split('/')
            .next()
            .and_then(|name| self.aggregates.get(name))
            .map(Aggregate::never_stale)
            .unwrap_or_default()
    }

    fn lock_queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_reviews(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, (Review, String)>> {
        self.reviews
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A copy of the queue's entry for `command`.
    fn entry(&self, command: &str) -> Result<Entry> {
        self.lock_queue()
            .entry(command)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("command {command}")))
    }

    /// The entry, when `by` is its author.
    fn owned(&self, command: &str, by: &str, gesture: &str) -> Result<Entry> {
        let entry = self.entry(command)?;
        if entry.by == by {
            Ok(entry)
        } else {
            Err(Error::Forbidden(format!(
                "{command} is {}'s to {gesture}",
                entry.by
            )))
        }
    }

    fn next_id(&self, prefix: &str) -> String {
        format!("{prefix}_{}", self.ids.fetch_add(1, Ordering::SeqCst))
    }

    fn notify_all(&self, messages: &[Message]) {
        for message in messages {
            self.notifier.notify(message);
        }
    }

    /// The manifest of a `DataCapability`, with or without its version.
    fn manifest(&self, id: &str) -> Result<&DataCapability> {
        let bare = id.split('@').next().unwrap_or(id);
        self.manifests
            .get(bare)
            .ok_or_else(|| Error::UnknownDataCapability(bare.to_string()))
    }

    /// Whether a persisted query is served: registered under this
    /// Capability with this hash. What the read crate asks before it runs
    /// anything.
    ///
    /// # Errors
    ///
    /// [`Error::UnregisteredQuery`].
    pub fn served(&self, capability: &str, sha256: &str) -> Result<()> {
        self.registry
            .served(capability, sha256)
            .map(|_| ())
            .ok_or_else(|| Error::UnregisteredQuery(format!("{capability} ({sha256})")))
    }

    /// The `dataVersion` now.
    ///
    /// # Errors
    ///
    /// The store's.
    pub async fn data_version(&self) -> Result<u64> {
        self.store.data_version().await
    }

    /// A command enters the queue, then the follow-ups its `Prepare` asked
    /// for ([`Prepared::then`]), in order, each by the same caller and
    /// through this same path: authorised, replayed on its idempotency key,
    /// prepared. They are not atomic with it: a follow-up refused undoes
    /// nothing, is listed in [`Submitted::then`] with why, and the ones
    /// after it are not submitted. A replay submits no follow-up.
    ///
    /// # Errors
    ///
    /// The first command's: [`Error::UnknownDataCapability`],
    /// [`Error::Forbidden`] (the caller, a missing idempotency key),
    /// [`Error::Refused`], or the store's. A follow-up's error is in
    /// [`Submitted::then`].
    pub async fn submit(&self, submission: Submission) -> Result<Submitted> {
        let caller = submission.caller.clone();
        let (mut submitted, then) = self.submit_one(submission, true).await?;
        for mut follow in then {
            follow.caller = caller.clone();
            let data_capability = follow.data_capability.clone();
            match self.submit_one(follow, false).await {
                Ok((one, _)) => submitted.then.push(FollowUp::Submitted(Box::new(one))),
                Err(error) => {
                    submitted.then.push(FollowUp::Refused {
                        data_capability,
                        error: error.to_string(),
                    });
                    break;
                }
            }
        }
        Ok(submitted)
    }

    /// One command enters the queue; its follow-ups come back unsubmitted.
    /// A follow-up (`leading` false) whose `Prepare` asks for follow-ups of
    /// its own is refused: one level only, so a chain always ends.
    async fn submit_one(
        &self,
        submission: Submission,
        leading: bool,
    ) -> Result<(Submitted, Vec<Submission>)> {
        let manifest = self.manifest(&submission.data_capability)?.clone();
        guard::authorise(&manifest, &submission.caller)?;
        if manifest.idempotency_key == Idempotency::Required && submission.idempotency_key.is_none()
        {
            return Err(Error::Forbidden(format!(
                "{} requires an idempotency key",
                manifest.data_capability
            )));
        }
        if let Some(key) = &submission.idempotency_key
            && let Some(existing) = self
                .store
                .by_idempotency_key(&manifest.data_capability, key)
                .await?
        {
            return Ok((self.replayed(&existing).await?, Vec::new()));
        }
        // What the author sent is what the manifest declares, not what the
        // `Prepare` rewrote it into.
        let undeclared = Self::undeclared_keys(&manifest, &submission.payload);
        let named = submission
            .target_id
            .clone()
            .or_else(|| Self::target_from_payload(&manifest, &submission.payload));
        let Prepared {
            payload,
            target_id,
            warnings: prepared,
            then,
            attachment,
        } = self.prepared(&manifest, &submission, named).await?;
        if !leading && !then.is_empty() {
            return Err(Error::Misconfigured(format!(
                "{} runs as a follow-up: its Prepare may not ask for follow-ups of its own ({} asked)",
                manifest.data_capability,
                then.len()
            )));
        }
        let target_id = target_id
            .or_else(|| Self::target_from_payload(&manifest, &payload))
            .unwrap_or_else(|| self.row_ids.fresh());
        let partition = format!("{}/{target_id}", manifest.target.aggregate);
        let row = self.store.row(&partition).await?;
        let command = self.next_id("c");
        let entry = Entry {
            command: command.clone(),
            data_capability: manifest.versioned(),
            by: submission.caller.by.clone(),
            partition: partition.clone(),
            position: 0,
            based_on: submission.based_on.clone().unwrap_or_default(),
            payload,
            effect: manifest.effect,
            touches: manifest.touches.clone(),
            mode: manifest.mode,
            projection: Projection::default(),
            state: State::Queued,
            confirmation: None,
            parked: None,
            at_head_since: None,
            requeued_from: None,
            idempotency_key: submission.idempotency_key.clone(),
            attachment,
        };
        self.writes_no_derived(&manifest, &entry)?;
        let Judged {
            mut warnings,
            mut messages,
            needs_review,
        } = self.judge(&manifest, &entry, &row, &submission.caller);
        warnings.splice(0..0, prepared);
        if !undeclared.is_empty() {
            warnings.push(format!(
                "payload key(s) {} not declared by {}",
                undeclared.join(", "),
                manifest.data_capability
            ));
        }
        let never_stale = self.never_stale(&partition);
        let (entry, queued) = self
            .lock_queue()
            .submit(entry, row, needs_review, &never_stale);
        messages.extend(queued);
        self.store.save(&entry).await?;
        self.notify_all(&messages);
        if entry.state == State::AwaitingConfirmation {
            warnings.push("a value is declared ahead: confirm or cancel".to_string());
        }
        let submitted = Submitted {
            command_id: command,
            partition,
            replayed: false,
            warnings,
            result: self.result_of(&entry),
            then: Vec::new(),
        };
        Ok((submitted, then))
    }

    /// Refuses an entry that writes a field its aggregate derives.
    fn writes_no_derived(&self, manifest: &DataCapability, entry: &Entry) -> Result<()> {
        let Some(aggregate) = self.aggregates.get(&manifest.target.aggregate) else {
            return Ok(());
        };
        let derived = guard::derived_touched(aggregate, entry);
        if derived.is_empty() {
            Ok(())
        } else {
            Err(Error::Forbidden(format!(
                "{} writes the derived field(s) {}: {} recomputes them",
                manifest.data_capability,
                derived.join(", "),
                aggregate.aggregate
            )))
        }
    }

    /// The submission as the `DataCapability`'s [`Prepare`] turns it — on
    /// the row it names as the commands pending leave it — or as it came
    /// when it has none.
    async fn prepared(
        &self,
        manifest: &DataCapability,
        submission: &Submission,
        target_id: Option<String>,
    ) -> Result<Prepared> {
        let Some((_, prepare)) = self
            .prepares
            .iter()
            .find(|(name, _)| *name == manifest.data_capability)
        else {
            return Ok(Prepared {
                payload: submission.payload.clone(),
                target_id,
                ..Prepared::default()
            });
        };
        let base = match &target_id {
            Some(id) => {
                self.projected_row(&format!("{}/{id}", manifest.target.aggregate))
                    .await?
            }
            None => BTreeMap::new(),
        };
        let lent = Lent {
            engine: self,
            failed: Mutex::new(None),
        };
        let context = Context {
            manifest,
            caller: &submission.caller,
            base: &base,
            now: self.clock.now(),
            now_millis: self.clock.now_millis(),
            ids: self.row_ids.as_ref(),
            shared: &self.shared,
            partitions: &lent,
            rows: self.rows.as_deref(),
        };
        let prepared = prepare
            .prepare(&context, &submission.payload, target_id.as_deref())
            .await;
        if let Some(failure) = lent
            .failed
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return Err(failure);
        }
        prepared.map_err(Error::Refused)
    }

    /// The row of `partition` as the commands pending in it leave it: the
    /// store's row, then each pending command's write, sets unioned.
    async fn projected_row(&self, partition: &str) -> Result<BTreeMap<String, Value>> {
        let row = self.store.row(partition).await?;
        let sets = partition
            .split('/')
            .next()
            .and_then(|aggregate| self.aggregates.get(aggregate))
            .map(Aggregate::sets)
            .unwrap_or_default();
        Ok(self.lock_queue().projected_row(partition, &row, &sets))
    }

    /// The payload's keys the manifest does not declare: nothing when it
    /// declares none, and the key naming the target is always declared.
    fn undeclared_keys(
        manifest: &DataCapability,
        payload: &BTreeMap<String, Value>,
    ) -> Vec<String> {
        if manifest.payload.is_empty() {
            return Vec::new();
        }
        let target = manifest
            .target
            .id
            .as_deref()
            .map(|id| id.trim_start_matches('$'));
        payload
            .keys()
            .filter(|key| !manifest.payload.contains_key(*key) && Some(key.as_str()) != target)
            .cloned()
            .collect()
    }

    /// The row's id named by the payload, when the manifest says which
    /// field carries it (`"id": "$accountId"`).
    fn target_from_payload(
        manifest: &DataCapability,
        payload: &BTreeMap<String, Value>,
    ) -> Option<String> {
        let key = manifest.target.id.as_deref()?.trim_start_matches('$');
        let value = payload.get(key)?;
        value
            .as_str()
            .map(ToString::to_string)
            .or_else(|| value.as_i64().map(|n| n.to_string()))
    }

    /// The answer to a submission an idempotency key already named.
    async fn replayed(&self, existing: &str) -> Result<Submitted> {
        let result = self.status(existing).await?;
        let partition = self
            .lock_queue()
            .entry(existing)
            .map_or_else(String::new, |e| e.partition.clone());
        Ok(Submitted {
            command_id: existing.to_string(),
            partition,
            replayed: true,
            warnings: Vec::new(),
            result,
            then: Vec::new(),
        })
    }

    /// The `DataGuard`'s judgement at enqueue: the invariants on the
    /// projected state warn, a critical field or a reviewing invariant
    /// opens a review.
    fn judge(
        &self,
        manifest: &DataCapability,
        entry: &Entry,
        row: &BTreeMap<String, Value>,
        caller: &Caller,
    ) -> Judged {
        let mut judged = Judged {
            warnings: Vec::new(),
            messages: Vec::new(),
            needs_review: false,
        };
        let Some(aggregate) = self.aggregates.get(&manifest.target.aggregate) else {
            return judged;
        };
        let projected = guard::projected(row, entry, &aggregate.sets());
        let verdict = guard::check(aggregate, &self.judges, &projected, entry);
        if !verdict.holds() {
            let at_risk: Vec<String> = verdict
                .rejecting
                .iter()
                .chain(verdict.reviewing.iter())
                .cloned()
                .collect();
            judged
                .warnings
                .push(format!("invariant(s) at risk: {}", at_risk.join(", ")));
            judged.messages.push(Message {
                violations: at_risk,
                ..Message::new(Kind::InvariantAtRisk, &[&entry.by], &entry.command)
            });
        }
        let review = guard::review_needed(aggregate, entry, caller);
        if !review.is_empty() || !verdict.reviewing.is_empty() {
            judged.needs_review = true;
            let review_id = self.open_review(aggregate, entry, row, &review);
            judged.warnings.push(format!("review {review_id} required"));
        }
        judged
    }

    /// Opens a review for `entry` on its first critical field, with the AI's
    /// proposal, and tells the approvers.
    fn open_review(
        &self,
        aggregate: &Aggregate,
        entry: &Entry,
        row: &BTreeMap<String, Value>,
        needed: &[String],
    ) -> String {
        let id = self.next_id("r");
        let field = entry.fields().first().cloned().unwrap_or_default();
        let approver = aggregate
            .fields
            .get(&field)
            .and_then(|f| f.review.clone())
            .unwrap_or_else(|| "role:approver".to_string());
        let candidates = vec![Candidate {
            command: entry.command.clone(),
            by: entry.by.clone(),
            value: entry.payload.get(&field).cloned().unwrap_or(Value::Null),
        }];
        let review = Review::open(
            &id,
            &entry.partition,
            &field,
            Base {
                version: entry.based_on.version,
                value: row.get(&field).cloned().unwrap_or(Value::Null),
            },
            candidates,
            &approver,
        )
        .with_proposal(self.proposer.as_ref());
        self.notifier.notify(&Message {
            reason: Some(if needed.is_empty() {
                "an invariant wants a review".to_string()
            } else {
                format!("critical: {}", needed.join(", "))
            }),
            actions: vec![Action::Approve, Action::Reject],
            ..Message::new(Kind::ReviewRequired, &[&approver], &entry.command)
        });
        self.lock_reviews()
            .insert(id.clone(), (review, entry.command.clone()));
        id
    }

    fn result_of(&self, entry: &Entry) -> CommandResult {
        let review_id = self
            .lock_reviews()
            .iter()
            .find(|(_, (review, command))| {
                *command == entry.command && review.decision.status == Status::Pending
            })
            .map(|(id, _)| id.clone());
        CommandResult {
            review_id,
            ..CommandResult::of(entry, None)
        }
    }

    /// Where a command stands: in the queue, or in the journal once final.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], or the store's.
    pub async fn status(&self, command: &str) -> Result<CommandResult> {
        if let Ok(entry) = self.entry(command)
            && !entry.state.is_final()
        {
            return Ok(self.result_of(&entry));
        }
        self.store
            .result(command)
            .await?
            .ok_or_else(|| Error::NotFound(format!("command {command}")))
    }

    /// The review with this id.
    #[must_use]
    pub fn review(&self, id: &str) -> Option<Review> {
        self.lock_reviews()
            .get(id)
            .map(|(review, _)| review.clone())
    }

    /// The last impact plan the Resolver built for `command` (contract J),
    /// once its head was looked at.
    #[must_use]
    pub fn plan(&self, command: &str) -> Option<ImpactPlan> {
        self.plans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(command)
            .cloned()
    }

    /// True when a human already approved a review of `command`: the
    /// Resolver does not ask twice.
    fn approved(&self, command: &str) -> bool {
        self.lock_reviews()
            .values()
            .any(|(review, of)| of == command && review.decision.status == Status::Approved)
    }

    /// The author accepts to overwrite what is declared ahead.
    ///
    /// # Errors
    ///
    /// [`Error::Forbidden`] for someone else, the queue's otherwise.
    pub async fn confirm(&self, command: &str, by: &str) -> Result<CommandResult> {
        self.owned(command, by, "confirm")?;
        let entry = self.lock_queue().confirm(command, by)?.clone();
        self.store.save(&entry).await?;
        Ok(self.result_of(&entry))
    }

    /// The author withdraws the command.
    ///
    /// # Errors
    ///
    /// [`Error::Forbidden`] for someone else, the queue's otherwise.
    pub async fn cancel(&self, command: &str, by: &str) -> Result<CommandResult> {
        self.owned(command, by, "cancel")?;
        let messages = self.lock_queue().cancel(command)?;
        let entry = self.entry(command)?;
        self.store.save(&entry).await?;
        let result = CommandResult::of(&entry, None);
        self.store.journal(&result).await?;
        self.notify_all(&messages);
        Ok(result)
    }

    /// A parked command's author asks for a recompute: back at the end,
    /// on the row as it is now.
    ///
    /// # Errors
    ///
    /// [`Error::Forbidden`] for someone else, the queue's otherwise.
    pub async fn requeue(&self, command: &str, by: &str) -> Result<Submitted> {
        let partition = self.owned(command, by, "requeue")?.partition;
        let row = self.store.row(&partition).await?;
        let never_stale = self.never_stale(&partition);
        let (fresh, messages) = self.lock_queue().requeue(command, row, &never_stale)?;
        let old = self.entry(command)?;
        self.store.save(&old).await?;
        self.store.save(&fresh).await?;
        self.notify_all(&messages);
        Ok(Submitted {
            command_id: fresh.command.clone(),
            partition,
            replayed: false,
            warnings: Vec::new(),
            result: self.result_of(&fresh),
            then: Vec::new(),
        })
    }

    /// A workflow freezes a partition for `ttl_seconds` — the aggregate's
    /// maximum when larger.
    ///
    /// # Errors
    ///
    /// The store's.
    pub async fn hold(&self, partition: &str, by: &str, ttl_seconds: Option<u64>) -> Result<Hold> {
        let aggregate = partition.split('/').next().unwrap_or(partition);
        let max = self
            .aggregates
            .get(aggregate)
            .map_or(60, Aggregate::max_hold_seconds);
        let ttl = ttl_seconds.map_or(max, |wanted| wanted.min(max));
        let id = self.next_id("w");
        let hold = self
            .lock_queue()
            .hold(&id, partition, by, ttl, self.clock.now());
        self.store.save_hold(&hold, true).await?;
        Ok(hold)
    }

    /// The owner renews the hold.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], or the store's.
    pub async fn renew(&self, hold: &str) -> Result<Hold> {
        let renewed = self.lock_queue().renew(hold, self.clock.now())?.clone();
        self.store.save_hold(&renewed, true).await?;
        Ok(renewed)
    }

    /// The owner releases the hold.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], or the store's.
    pub async fn release(&self, hold: &str) -> Result<()> {
        let released = self.lock_queue().release(hold)?;
        self.store.save_hold(&released, false).await
    }

    /// An approver decides a review: approved, the command goes on;
    /// rejected, it is rejected.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], the review's ([`Error::Forbidden`],
    /// [`Error::NotNow`]), or the store's.
    pub async fn decide(
        &self,
        review: &str,
        by: &str,
        roles: &[String],
        approved: bool,
    ) -> Result<CommandResult> {
        let command = self.decided(review, by, roles, approved)?;
        let entry = self.lock_queue().reviewed(&command, approved)?.clone();
        self.store.save(&entry).await?;
        if entry.state == State::Rejected {
            let result = CommandResult {
                violations: vec![format!("review {review} rejected")],
                ..CommandResult::of(&entry, None)
            };
            self.store.journal(&result).await?;
            self.notifier.notify(&Message {
                reason: Some(format!("review {review} rejected by {by}")),
                ..Message::new(Kind::CommandRejected, &[&entry.by], &command)
            });
            return Ok(result);
        }
        Ok(self.result_of(&entry))
    }

    /// Moves the review, and names the command it is about.
    fn decided(&self, review: &str, by: &str, roles: &[String], approved: bool) -> Result<String> {
        let mut reviews = self.lock_reviews();
        let (found, command) = reviews
            .get_mut(review)
            .ok_or_else(|| Error::NotFound(format!("review {review}")))?;
        found.decide(by, roles, approved)?;
        let command = command.clone();
        drop(reviews);
        Ok(command)
    }

    /// Drives every partition once: time passes, each head that is ready is
    /// applied, parked heads are set aside. What was applied, rejected or
    /// expired comes back.
    ///
    /// # Errors
    ///
    /// The store's.
    pub async fn run_once(&self) -> Result<Vec<CommandResult>> {
        let now = self.clock.now();
        let (expired, holds_now) = self.ticked(now);
        let mut results = Vec::new();
        for entry in expired {
            self.store.save(&entry).await?;
            let result = CommandResult::of(&entry, None);
            self.store.journal(&result).await?;
            results.push(result);
        }
        // Holds that expired are gone from the queue; the store learns it.
        for hold in self.store.holds().await? {
            if !holds_now.iter().any(|h| h.id == hold.id) {
                self.store.save_hold(&hold, false).await?;
            }
        }
        for partition in self.partitions() {
            for _ in 0..HEADS_PER_RUN {
                let (step, messages) = self.lock_queue().advance(&partition, now);
                self.notify_all(&messages);
                match step {
                    Step::Ready(head) => {
                        let outcome = self.apply_head(&head).await?;
                        let go_on = matches!(outcome.status, State::Applied | State::Rejected);
                        results.push(outcome);
                        if !go_on {
                            break;
                        }
                    }
                    Step::Parked(head) => {
                        if let Ok(entry) = self.entry(&head) {
                            self.store.save(&entry).await?;
                        }
                    }
                    Step::Empty | Step::Blocked(_) | Step::Waiting(_) => break,
                }
            }
        }
        Ok(results)
    }

    /// Time passes on the queue: what expired, and the holds still alive.
    fn ticked(&self, now: u64) -> (Vec<Entry>, Vec<Hold>) {
        let mut queue = self.lock_queue();
        let messages = queue.tick(now);
        let expired: Vec<Entry> = queue
            .entries()
            .iter()
            .filter(|e| e.state == State::Expired)
            .cloned()
            .collect();
        let holds = queue.holds().to_vec();
        drop(queue);
        self.notify_all(&messages);
        (expired, holds)
    }

    /// The partitions with something pending, each once, sorted.
    fn partitions(&self) -> Vec<String> {
        let queue = self.lock_queue();
        let mut seen: Vec<String> = queue
            .entries()
            .iter()
            .filter(|e| !e.state.is_final())
            .map(|e| e.partition.clone())
            .collect();
        drop(queue);
        seen.sort();
        seen.dedup();
        seen
    }

    /// The single applier's step on `head`: classified at execution,
    /// planned when it mutates existing data, checked on the real state,
    /// applied or not.
    async fn apply_head(&self, head: &str) -> Result<CommandResult> {
        let entry = self.entry(head)?;
        let manifest = self.manifest(&entry.data_capability)?.clone();
        let row = self.store.row(&entry.partition).await?;
        let aggregate = self.aggregates.get(&manifest.target.aggregate);
        let sets = aggregate.map(Aggregate::sets).unwrap_or_default();
        let projected = guard::projected(&row, &entry, &sets);
        let verdict = aggregate.map_or_else(Verdict::default, |aggregate| {
            guard::check(aggregate, &self.judges, &projected, &entry)
        });
        let collision = self.store.collision(&entry).await?;
        let classification = guard::classify(&manifest, &collision);
        let decision = match (&classification, aggregate) {
            (Classification::PureAddition, _) | (_, None) => {
                if verdict.rejecting.is_empty() {
                    Decision::Auto
                } else {
                    Decision::Rejected
                }
            }
            (Classification::Mutation | Classification::Reclassified(_), Some(aggregate)) => {
                let incoming = self.store.incoming(&entry.partition).await?;
                if let Classification::Reclassified(found) = &classification {
                    self.notifier.notify(&Message {
                        reason: Some(format!(
                            "declared a pure addition, found not to be one: {found:?} — routed to the Resolver"
                        )),
                        ..Message::new(Kind::InvariantAtRisk, &[&entry.by], head)
                    });
                }
                let plan = self.planned(&entry, aggregate, &verdict, &incoming);
                let decision = plan.decision;
                self.plans
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(head.to_string(), plan);
                match decision {
                    Decision::HumanReview if self.approved(head) => Decision::Auto,
                    other => other,
                }
            }
        };
        match decision {
            Decision::Auto | Decision::Confirm => self.applied(&entry, projected).await,
            Decision::Rejected => {
                self.rejected(&entry, &verdict.rejecting, &rejection(&verdict))
                    .await
            }
            Decision::HumanReview => {
                let review_id = aggregate.map_or_else(String::new, |aggregate| {
                    self.open_review(aggregate, &entry, &row, &[])
                });
                let waiting = self.lock_queue().await_review(head)?.clone();
                self.store.save(&waiting).await?;
                Ok(CommandResult {
                    review_id: Some(review_id),
                    ..CommandResult::of(&waiting, None)
                })
            }
            Decision::Blocked => Ok(CommandResult::of(&entry, None)),
        }
    }

    /// The Resolver's plan for a mutation, from the queue's holds and what
    /// is behind the entry.
    fn planned(
        &self,
        entry: &Entry,
        aggregate: &Aggregate,
        verdict: &Verdict,
        incoming: &[(String, u64)],
    ) -> ImpactPlan {
        let queue = self.lock_queue();
        // Every hold: the Resolver keeps the ones on the partition, at or
        // before the entry.
        let holds: Vec<Hold> = queue.holds().to_vec();
        let pending_behind: Vec<String> = queue
            .entries()
            .iter()
            .filter(|e| {
                e.partition == entry.partition && e.position > entry.position && !e.state.is_final()
            })
            .map(|e| e.command.clone())
            .collect();
        drop(queue);
        resolver::plan(
            entry,
            &Situation {
                aggregate,
                registry: &self.registry,
                holds: &holds,
                pending_behind: &pending_behind,
                incoming,
                verdict,
                // A head the queue let through has nothing left to confirm:
                // it was confirmed, or nothing was declared ahead of it.
                confirmed: true,
            },
        )
    }

    /// Applies the head: the write, the version, the journal, the word. A
    /// store that refuses the write (a projection in its transaction) has
    /// rolled it back: the head is rejected, with the refusal's reason.
    async fn applied(
        &self,
        entry: &Entry,
        projected: BTreeMap<String, Value>,
    ) -> Result<CommandResult> {
        let write = Write {
            partition: entry.partition.clone(),
            values: if entry.effect == Effect::Delete {
                BTreeMap::new()
            } else {
                projected
            },
            tombstone: entry.effect == Effect::Delete,
        };
        let mut result = CommandResult {
            status: State::Applied,
            attachment: entry.attachment.clone(),
            ..CommandResult::of(entry, None)
        };
        let version = match self.store.apply(&write, &result).await {
            Err(Error::Refused(refusal)) => {
                return self.rejected(entry, &[], &refusal.message).await;
            }
            applied => applied?,
        };
        for project in self.projects.get(entry.aggregate()).into_iter().flatten() {
            project.observe(&write);
        }
        result.data_version = Some(version);
        let messages = self.lock_queue().applied(&entry.command, version)?;
        let applied = self.entry(&entry.command)?;
        self.store.save(&applied).await?;
        self.notify_all(&messages);
        Ok(result)
    }

    /// Rejects the head: these invariants violated, the author told why.
    async fn rejected(
        &self,
        entry: &Entry,
        violations: &[String],
        reason: &str,
    ) -> Result<CommandResult> {
        let messages = self
            .lock_queue()
            .rejected(&entry.command, violations, reason)?;
        let rejected = self.entry(&entry.command)?;
        self.store.save(&rejected).await?;
        let result = CommandResult {
            violations: violations.to_vec(),
            ..CommandResult::of(&rejected, None)
        };
        self.store.journal(&result).await?;
        self.notify_all(&messages);
        Ok(result)
    }

    /// The caller shape the server builds from a request's headers.
    #[must_use]
    pub fn caller(by: &str, from: &str, permissions: &[&str], roles: &[&str]) -> Caller {
        Caller {
            by: by.to_string(),
            from: from.to_string(),
            permissions: permissions.iter().map(|p| (*p).to_string()).collect(),
            roles: roles.iter().map(|r| (*r).to_string()).collect(),
        }
    }
}

/// Why the real state rejects a head: its invariants, or the Resolver.
fn rejection(verdict: &Verdict) -> String {
    if verdict.rejecting.is_empty() {
        "the Resolver rejected it".to_string()
    } else {
        format!(
            "invariant(s) violated on the real state: {}",
            verdict.rejecting.join(", ")
        )
    }
}

/// A `basedOn` at this version, having read these values.
#[must_use]
pub fn based_on(version: u64, values: &[(&str, Value)]) -> BasedOn {
    BasedOn {
        version,
        values: values
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{FixedClock, MemoryNotifier, MemoryStore};
    use crate::messages::Kind;
    use crate::plugins::{Plugin, Refusal, RefusalKind};
    use crate::store::Collision;

    const ACCOUNT: &str = r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"normal"},"tags":{"kind":"authoritative","criticality":"low"},"status":{"kind":"authoritative","criticality":"critical","review":"role:credit-manager"}},
        "invariants":[{"id":"limit-under-ceiling","rule":"creditLimit <= 10000","onViolation":"reject"}],
        "relations":[{"from":"Payment.accountId","onDelete":"restrict"}],"holds":{"maxTtl":"1m","renewable":true}}"#;
    const LIMIT: &str = r#"{"dataCapability":"credit.requestLimitChange","owner":"dataguard","version":2,"effect":"update","target":{"aggregate":"Account","id":"$accountId"},"touches":["Account.creditLimit"],"mode":"confirm_on_stale","permissions":["credit.limit.write"],"callableBy":["credit.*"],"idempotencyKey":"optional"}"#;
    const CLOSE: &str = r#"{"dataCapability":"credit.closeAccount","owner":"dataguard","version":1,"effect":"delete","target":{"aggregate":"Account","id":"$accountId"},"mode":"overwrite","callableBy":["*"]}"#;
    const ADD: &str = r#"{"dataCapability":"credit.addPayment","owner":"dataguard","version":1,"effect":"insert","target":{"aggregate":"Payment"},"callableBy":["*"],"idempotencyKey":"required"}"#;
    const PAYMENT: &str = r#"{"aggregate":"Payment","fields":{"amount":{"kind":"authoritative","criticality":"normal"}},"invariants":[],"relations":[]}"#;
    const REVIEWED: &str = r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"normal"}},"invariants":[],"relations":[{"from":"Invoice.accountId","onDelete":"review"}]}"#;
    const STATUS: &str = r#"{"dataCapability":"credit.setStatus","owner":"dataguard","version":1,"effect":"update","target":{"aggregate":"Account","id":"$accountId"},"touches":["Account.status"],"mode":"overwrite","callableBy":["*"]}"#;

    /// What a product codes beyond the rules its aggregate writes down:
    /// here, a raise past 7000 wants a fresh risk score.
    struct RiskFresh;

    impl Judge for RiskFresh {
        fn violated(&self, state: &BTreeMap<String, Value>, _entry: &Entry) -> Vec<String> {
            if state
                .get("creditLimit")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                > 7000
            {
                vec!["risk-fresh".to_string()]
            } else {
                Vec::new()
            }
        }
    }

    struct Plant {
        engine: Engine,
        store: Arc<MemoryStore>,
        notifier: Arc<MemoryNotifier>,
        clock: Arc<FixedClock>,
    }

    fn plant(store: MemoryStore) -> Plant {
        let store = Arc::new(store);
        let notifier = Arc::new(MemoryNotifier::default());
        let clock = Arc::new(FixedClock::at(1000));
        let engine = Engine::new(store.clone(), notifier.clone(), clock.clone())
            .with_aggregate(Aggregate::parse(ACCOUNT).expect("I"))
            .with_data_capability(DataCapability::parse(LIMIT).expect("F"))
            .with_data_capability(DataCapability::parse(CLOSE).expect("F"))
            .with_data_capability(DataCapability::parse(ADD).expect("F"))
            .with_data_capability(DataCapability::parse(STATUS).expect("F"));
        Plant {
            engine,
            store,
            notifier,
            clock,
        }
    }

    fn account(limit: i64) -> MemoryStore {
        MemoryStore::default().with_row(
            "Account/881",
            BTreeMap::from([("creditLimit".to_string(), Value::from(limit))]),
        )
    }

    fn carol() -> Caller {
        Engine::caller(
            "carol",
            "credit.LimitReviewForm",
            &["credit.limit.write"],
            &[],
        )
    }

    fn limit(by: &str, new_limit: i64, read: i64) -> Submission {
        Submission {
            data_capability: "credit.requestLimitChange".to_string(),
            payload: BTreeMap::from([
                ("accountId".to_string(), Value::from("881")),
                ("creditLimit".to_string(), Value::from(new_limit)),
            ]),
            target_id: None,
            based_on: Some(based_on(10, &[("creditLimit", Value::from(read))])),
            idempotency_key: None,
            caller: Caller {
                by: by.to_string(),
                ..carol()
            },
        }
    }

    const BOARD: &str = r#"{"aggregate":"Board","fields":{"tags":{"kind":"authoritative","criticality":"low","merge":"set"},"note":{"kind":"authoritative","criticality":"low","merge":"last-writer"},"score":{"kind":"derived","criticality":"normal"}},"invariants":[{"id":"score-positive","rule":"score >= 0","onViolation":"reject"}]}"#;

    fn board_capability(name: &str, field: &str, extra: &str) -> DataCapability {
        DataCapability::parse(&format!(
            r#"{{"dataCapability":"board.{name}","owner":"board","version":1,"effect":"update","target":{{"aggregate":"Board","id":"$boardId"}},"touches":["Board.{field}"],"callableBy":["*"]{extra}}}"#
        ))
        .expect("F")
    }

    fn board_plant(row: BTreeMap<String, Value>) -> Plant {
        let store = Arc::new(MemoryStore::default().with_row("Board/1", row));
        let notifier = Arc::new(MemoryNotifier::default());
        let clock = Arc::new(FixedClock::at(1000));
        let engine = Engine::new(store.clone(), notifier.clone(), clock.clone())
            .with_aggregate(Aggregate::parse(BOARD).expect("I"))
            .with_data_capability(board_capability("tag", "tags", ""))
            .with_data_capability(board_capability("note", "note", ""))
            .with_data_capability(board_capability("score", "score", ""))
            .with_data_capability(board_capability(
                "rename",
                "note",
                r#","payload":{"boardId":"string","note":"string"}"#,
            ));
        Plant {
            engine,
            store,
            notifier,
            clock,
        }
    }

    fn on_board(capability: &str, by: &str, field: &str, value: Value, read: Value) -> Submission {
        Submission {
            data_capability: format!("board.{capability}"),
            payload: BTreeMap::from([
                ("boardId".to_string(), Value::from("1")),
                (field.to_string(), value),
            ]),
            target_id: None,
            based_on: Some(based_on(1, &[(field, read)])),
            idempotency_key: None,
            caller: Caller {
                by: by.to_string(),
                ..carol()
            },
        }
    }

    #[tokio::test]
    async fn a_command_writing_a_derived_field_is_forbidden() {
        let plant = board_plant(BTreeMap::new());
        let refused = plant
            .engine
            .submit(on_board(
                "score",
                "alice",
                "score",
                Value::from(3),
                Value::Null,
            ))
            .await;
        assert!(
            matches!(&refused, Err(Error::Forbidden(why)) if why.contains("derived field(s) score")),
            "{refused:?}"
        );
        assert!(
            plant.engine.run_once().await.expect("ran").is_empty(),
            "nothing was queued"
        );
    }

    #[tokio::test]
    async fn a_set_field_is_never_asked_and_its_writes_union() {
        let plant = board_plant(BTreeMap::from([(
            "tags".to_string(),
            serde_json::json!(["a"]),
        )]));
        for (by, tags) in [
            ("alice", serde_json::json!(["b"])),
            ("bob", serde_json::json!(["c", "a"])),
        ] {
            let submitted = plant
                .engine
                .submit(on_board("tag", by, "tags", tags, serde_json::json!(["x"])))
                .await
                .expect("queued");
            assert_eq!(submitted.result.status, State::Queued, "{by} is not asked");
            assert!(submitted.warnings.is_empty(), "{:?}", submitted.warnings);
        }
        plant.engine.run_once().await.expect("ran");
        plant.engine.run_once().await.expect("ran");
        assert_eq!(
            plant.store.row_now("Board/1")["tags"],
            serde_json::json!(["a", "b", "c"])
        );
    }

    #[tokio::test]
    async fn a_last_writer_field_is_never_asked_and_the_last_write_wins() {
        let plant = board_plant(BTreeMap::from([("note".to_string(), Value::from("old"))]));
        for (by, note) in [("alice", "first"), ("bob", "second")] {
            let submitted = plant
                .engine
                .submit(on_board(
                    "note",
                    by,
                    "note",
                    Value::from(note),
                    Value::from("stale"),
                ))
                .await
                .expect("queued");
            assert_eq!(submitted.result.status, State::Queued, "{by} is not asked");
        }
        plant.engine.run_once().await.expect("ran");
        plant.engine.run_once().await.expect("ran");
        assert_eq!(
            plant.store.row_now("Board/1")["note"],
            Value::from("second")
        );
    }

    #[tokio::test]
    async fn a_payload_key_the_manifest_does_not_declare_is_a_warning() {
        let plant = board_plant(BTreeMap::new());
        let mut submission = on_board("rename", "alice", "note", Value::from("n"), Value::Null);
        let clean = plant
            .engine
            .submit(submission.clone())
            .await
            .expect("queued");
        assert!(clean.warnings.is_empty(), "{:?}", clean.warnings);
        submission
            .payload
            .insert("colour".to_string(), Value::from("red"));
        let warned = plant.engine.submit(submission).await.expect("queued");
        assert_eq!(
            warned.warnings,
            ["payload key(s) colour not declared by board.rename"]
        );
        let undeclared = plant
            .engine
            .submit(on_board(
                "note",
                "bob",
                "note",
                Value::from("n"),
                Value::Null,
            ))
            .await
            .expect("queued");
        assert!(
            undeclared.warnings.is_empty(),
            "a manifest declaring no payload warns of nothing: {:?}",
            undeclared.warnings
        );
    }

    #[tokio::test]
    async fn load_refuses_a_manifest_listing_an_invariant_its_aggregate_lacks() {
        let fitting = board_plant(BTreeMap::new());
        assert_eq!(fitting.engine.load().await, Ok(()));
        let declared = board_capability("ok", "note", r#","invariants":["score-positive"]"#);
        let typo = board_capability("typo", "note", r#","invariants":["score-positiv"]"#);
        let store = Arc::new(MemoryStore::default());
        let notifier = Arc::new(MemoryNotifier::default());
        let clock = Arc::new(FixedClock::at(1000));
        let base = || {
            Engine::new(store.clone(), notifier.clone(), clock.clone())
                .with_aggregate(Aggregate::parse(BOARD).expect("I"))
        };
        assert_eq!(base().with_data_capability(declared).load().await, Ok(()));
        let refused = base().with_data_capability(typo).load().await;
        assert!(
            matches!(&refused, Err(Error::Misconfigured(why)) if why.contains("score-positiv ") && why.contains("Board")),
            "{refused:?}"
        );
        let orphan = Engine::new(store.clone(), notifier.clone(), clock.clone())
            .with_data_capability(board_capability(
                "o",
                "note",
                r#","invariants":["score-positive"]"#,
            ))
            .load()
            .await;
        assert!(
            matches!(orphan, Err(Error::Misconfigured(_))),
            "an aggregate the engine does not know declares nothing"
        );
    }

    #[tokio::test]
    async fn a_command_on_a_fresh_base_is_queued_applied_and_versioned() {
        let plant = plant(account(5000));
        let submitted = plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        assert_eq!(submitted.partition, "Account/881");
        assert_eq!(submitted.result.status, State::Queued);
        assert!(submitted.warnings.is_empty(), "{:?}", submitted.warnings);
        let applied = plant.engine.run_once().await.expect("ran");
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].status, State::Applied);
        assert_eq!(applied[0].data_version, Some(1));
        assert_eq!(
            plant.store.row_now("Account/881")["creditLimit"],
            Value::from(8000)
        );
        let status = plant
            .engine
            .status(&submitted.command_id)
            .await
            .expect("in the journal");
        assert_eq!(status.data_version, Some(1));
        let kinds: Vec<Kind> = plant
            .notifier
            .to("alice")
            .iter()
            .map(|m| m.message)
            .collect();
        assert_eq!(kinds, [Kind::CommandQueued, Kind::CommandApplied]);
        assert!(
            plant
                .notifier
                .to("infrastructure")
                .iter()
                .any(|m| m.message == Kind::CommandApplied)
        );
    }

    #[tokio::test]
    // The contract's scenario, whole: its length is the scenario's.
    #[allow(clippy::too_many_lines)]
    async fn the_contract_s_scenario_end_to_end() {
        // Alice declares 8000, Bob 3000 on the same base: Bob is asked.
        let plant = plant(account(5000));
        let alice = plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        let bob = plant
            .engine
            .submit(limit("bob", 3000, 5000))
            .await
            .expect("queued");
        assert_eq!(bob.result.status, State::AwaitingConfirmation);
        assert!(
            bob.warnings.iter().any(|w| w.contains("declared ahead")),
            "{:?}",
            bob.warnings
        );
        // Carol, behind both, confirms at once.
        let carol = plant
            .engine
            .submit(limit("carol", 4500, 5000))
            .await
            .expect("queued");
        plant
            .engine
            .confirm(&carol.command_id, "carol")
            .await
            .expect("confirmed");
        assert!(matches!(
            plant.engine.confirm(&carol.command_id, "mallory").await,
            Err(Error::Forbidden(_))
        ));
        // One run: Alice applied, Bob waits at the head for his grace, the
        // partition with him. The grace over, a run parks Bob and applies
        // Carol after.
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(
            results.iter().map(|r| r.status).collect::<Vec<_>>(),
            [State::Applied],
            "Alice alone: Bob holds the partition while he may still confirm"
        );
        assert_eq!(
            plant
                .engine
                .status(&bob.command_id)
                .await
                .expect("waiting")
                .status,
            State::AwaitingConfirmation
        );
        plant.clock.set(1000 + crate::queue::CONFIRM_GRACE_SECONDS);
        let results = plant.engine.run_once().await.expect("ran");
        let statuses: Vec<(String, State)> = results
            .iter()
            .map(|r| (r.command_id.clone(), r.status))
            .collect();
        assert_eq!(statuses, [(carol.command_id.clone(), State::Applied)]);
        assert_eq!(
            plant
                .engine
                .status(&alice.command_id)
                .await
                .expect("journaled")
                .status,
            State::Applied
        );
        assert_eq!(
            plant.store.row_now("Account/881")["creditLimit"],
            Value::from(4500)
        );
        assert_eq!(
            plant
                .engine
                .status(&bob.command_id)
                .await
                .expect("parked")
                .status,
            State::Parked
        );
        assert!(
            plant
                .notifier
                .to("bob")
                .iter()
                .any(|m| m.message == Kind::Parked)
        );
        assert!(
            plant
                .notifier
                .to("bob")
                .iter()
                .any(|m| m.message == Kind::AheadResolved)
        );
        // Bob asks for a recompute: back at the end, on 4500, nobody ahead.
        let again = plant
            .engine
            .requeue(&bob.command_id, "bob")
            .await
            .expect("requeued");
        assert_eq!(
            again.result.status,
            State::AwaitingConfirmation,
            "his base moved: 5000 read, 4500 now"
        );
        plant
            .engine
            .confirm(&again.command_id, "bob")
            .await
            .expect("confirmed");
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Applied);
        assert_eq!(
            plant.store.row_now("Account/881")["creditLimit"],
            Value::from(3000)
        );
        assert_eq!(
            plant.engine.data_version().await.expect("read"),
            3,
            "one global sequence"
        );
    }

    #[tokio::test]
    async fn an_author_who_confirms_during_the_grace_is_applied_not_parked() {
        let plant = plant(account(5000));
        plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        let bob = plant
            .engine
            .submit(limit("bob", 3000, 5000))
            .await
            .expect("asked");
        plant
            .engine
            .run_once()
            .await
            .expect("ran: alice applied, bob at the head");
        plant.clock.set(1000 + 10);
        plant
            .engine
            .confirm(&bob.command_id, "bob")
            .await
            .expect("in time, over HTTP");
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(
            results.iter().map(|r| r.status).collect::<Vec<_>>(),
            [State::Applied]
        );
        assert_eq!(
            plant.store.row_now("Account/881")["creditLimit"],
            Value::from(3000)
        );
        assert!(
            !plant
                .notifier
                .to("bob")
                .iter()
                .any(|m| m.message == Kind::Parked),
            "never parked"
        );
    }

    #[tokio::test]
    async fn a_parked_command_left_alone_expires_after_a_day() {
        let plant = plant(account(5000));
        plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        let bob = plant
            .engine
            .submit(limit("bob", 3000, 5000))
            .await
            .expect("queued");
        plant.engine.run_once().await.expect("ran");
        plant.clock.set(1000 + crate::queue::CONFIRM_GRACE_SECONDS);
        plant.engine.run_once().await.expect("ran");
        plant
            .clock
            .set(1000 + crate::queue::CONFIRM_GRACE_SECONDS + crate::queue::PARKED_TTL_SECONDS);
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(
            results.iter().map(|r| r.status).collect::<Vec<_>>(),
            [State::Expired]
        );
        assert_eq!(
            plant
                .engine
                .status(&bob.command_id)
                .await
                .expect("journaled")
                .status,
            State::Expired
        );
        assert!(
            plant
                .notifier
                .to("bob")
                .iter()
                .any(|m| m.message == Kind::Expired)
        );
    }

    #[tokio::test]
    async fn an_invariant_is_warned_at_enqueue_and_decides_at_application() {
        let plant = plant(account(5000));
        let over = plant
            .engine
            .submit(limit("alice", 20_000, 5000))
            .await
            .expect("queued, warned");
        assert!(
            over.warnings[0].contains("limit-under-ceiling"),
            "{:?}",
            over.warnings
        );
        let warned = plant
            .notifier
            .to("alice")
            .into_iter()
            .find(|m| m.message == Kind::InvariantAtRisk)
            .expect("warned");
        assert_eq!(warned.violations, ["limit-under-ceiling"]);
        assert!(
            plant
                .notifier
                .to("alice")
                .iter()
                .any(|m| m.message == Kind::InvariantAtRisk)
        );
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Rejected);
        assert_eq!(results[0].violations, ["limit-under-ceiling"]);
        assert_eq!(
            plant.store.row_now("Account/881")["creditLimit"],
            Value::from(5000),
            "untouched"
        );
        assert!(
            plant
                .notifier
                .to("alice")
                .iter()
                .any(|m| m.message == Kind::CommandRejected)
        );
    }

    #[tokio::test]
    async fn a_caller_outside_the_namespace_or_without_the_permission_is_refused() {
        let plant = plant(account(5000));
        let elsewhere = Submission {
            caller: Engine::caller("x", "orders.Form", &["credit.limit.write"], &[]),
            ..limit("x", 1, 5000)
        };
        assert!(matches!(
            plant.engine.submit(elsewhere).await,
            Err(Error::Forbidden(_))
        ));
        let unknown = Submission {
            data_capability: "credit.nothing".to_string(),
            ..limit("x", 1, 5000)
        };
        assert!(matches!(
            plant.engine.submit(unknown).await,
            Err(Error::UnknownDataCapability(_))
        ));
    }

    #[tokio::test]
    async fn a_pure_addition_takes_the_fast_path_and_an_idempotency_key_replays() {
        let plant = plant(MemoryStore::default());
        let add = |key: Option<&str>| Submission {
            data_capability: "credit.addPayment@1".to_string(),
            payload: BTreeMap::from([("amount".to_string(), Value::from(12))]),
            target_id: None,
            based_on: None,
            idempotency_key: key.map(ToString::to_string),
            caller: Engine::caller("alice", "credit.Form", &[], &[]),
        };
        assert!(
            matches!(
                plant.engine.submit(add(None)).await,
                Err(Error::Forbidden(_))
            ),
            "a key is required"
        );
        let first = plant.engine.submit(add(Some("k1"))).await.expect("queued");
        assert!(
            first.partition.starts_with("Payment/new_"),
            "{}",
            first.partition
        );
        let again = plant
            .engine
            .submit(add(Some("k1")))
            .await
            .expect("replayed");
        assert!(again.replayed);
        assert_eq!(again.command_id, first.command_id);
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, State::Applied);
        assert_eq!(
            plant.store.row_now(&first.partition)["amount"],
            Value::from(12)
        );
    }

    #[tokio::test]
    async fn an_addition_that_collides_is_reclassified_and_planned_not_failed() {
        let plant = plant(MemoryStore::default().colliding(
            "credit.addPayment",
            Collision {
                unique_key: Some("Payment.reference".to_string()),
                ..Collision::default()
            },
        ));
        let submission = Submission {
            data_capability: "credit.addPayment".to_string(),
            payload: BTreeMap::from([("amount".to_string(), Value::from(12))]),
            target_id: Some("p1".to_string()),
            based_on: None,
            idempotency_key: Some("k".to_string()),
            caller: Engine::caller("alice", "credit.Form", &[], &[]),
        };
        plant.engine.submit(submission).await.expect("queued");
        let results = plant.engine.run_once().await.expect("ran");
        // No aggregate is registered for Payment: the plan cannot be built,
        // and the invariants alone decide — applied.
        assert_eq!(results[0].status, State::Applied);
    }

    #[tokio::test]
    async fn a_deletion_under_a_restrict_relation_stays_blocked_in_its_place() {
        let plant = plant(
            account(5000).pointed_at("Account/881", vec![("Payment.accountId".to_string(), 42)]),
        );
        let close = Submission {
            data_capability: "credit.closeAccount".to_string(),
            payload: BTreeMap::from([("accountId".to_string(), Value::from("881"))]),
            target_id: None,
            based_on: None,
            idempotency_key: None,
            caller: Engine::caller("alice", "credit.Form", &[], &[]),
        };
        let submitted = plant.engine.submit(close).await.expect("queued");
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Queued, "blocked keeps its place");
        assert_eq!(
            plant
                .engine
                .status(&submitted.command_id)
                .await
                .expect("pending")
                .status,
            State::Queued
        );
        assert!(!plant.store.is_tombstoned("Account/881"));
    }

    #[tokio::test]
    async fn a_deletion_with_nothing_pointing_archives_with_a_tombstone() {
        let plant = plant(account(5000));
        let close = Submission {
            data_capability: "credit.closeAccount".to_string(),
            payload: BTreeMap::from([("accountId".to_string(), Value::from("881"))]),
            target_id: None,
            based_on: None,
            idempotency_key: None,
            caller: Engine::caller("alice", "credit.Form", &[], &[]),
        };
        plant.engine.submit(close).await.expect("queued");
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Applied);
        assert!(plant.store.is_tombstoned("Account/881"));
    }

    #[tokio::test]
    async fn a_critical_field_waits_for_its_approver_and_the_ai_only_proposes() {
        let plant = plant(account(5000));
        let set = Submission {
            data_capability: "credit.setStatus".to_string(),
            payload: BTreeMap::from([
                ("accountId".to_string(), Value::from("881")),
                ("status".to_string(), Value::from("closed")),
            ]),
            target_id: None,
            based_on: None,
            idempotency_key: None,
            caller: Engine::caller("alice", "credit.Form", &[], &[]),
        };
        let submitted = plant.engine.submit(set).await.expect("queued for review");
        assert_eq!(submitted.result.status, State::AwaitingReview);
        let review_id = submitted.result.review_id.clone().expect("a review");
        assert!(
            submitted
                .warnings
                .contains(&format!("review {review_id} required")),
            "the warning names the review opened: {:?}",
            submitted.warnings
        );
        let asked = plant
            .notifier
            .to("role:credit-manager")
            .into_iter()
            .find(|m| m.message == Kind::ReviewRequired)
            .expect("the approver is told");
        assert_eq!(
            asked.reason.as_deref(),
            Some("critical: status (role:credit-manager)")
        );
        assert_eq!(asked.actions, [Action::Approve, Action::Reject]);
        // Another author's plain command waits on no review of anyone's.
        let alice = plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        assert_eq!(alice.result.review_id, None, "someone else's review");
        assert!(
            plant.engine.run_once().await.expect("ran").is_empty(),
            "waits"
        );
        let review = plant.engine.review(&review_id).expect("open");
        assert!(review.proposal.is_none(), "no proposer wired");
        assert!(matches!(
            plant
                .engine
                .decide(&review_id, "carol", &["role:clerk".to_string()], true)
                .await,
            Err(Error::Forbidden(_))
        ));
        plant
            .engine
            .decide(
                &review_id,
                "dana",
                &["role:credit-manager".to_string()],
                true,
            )
            .await
            .expect("approved");
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Applied);
        assert_eq!(
            plant.store.row_now("Account/881")["status"],
            Value::from("closed")
        );
        assert!(plant.engine.review("r_nope").is_none());
    }

    #[tokio::test]
    async fn a_rejected_review_rejects_the_command() {
        let plant = plant(account(5000));
        let set = Submission {
            data_capability: "credit.setStatus".to_string(),
            payload: BTreeMap::from([
                ("accountId".to_string(), Value::from("881")),
                ("status".to_string(), Value::from("x")),
            ]),
            target_id: None,
            based_on: None,
            idempotency_key: None,
            caller: Engine::caller("alice", "credit.Form", &[], &[]),
        };
        let submitted = plant.engine.submit(set).await.expect("queued");
        let review = submitted.result.review_id.expect("a review");
        let result = plant
            .engine
            .decide(&review, "dana", &["role:credit-manager".to_string()], false)
            .await
            .expect("rejected");
        assert_eq!(result.status, State::Rejected);
        assert_eq!(result.violations, [format!("review {review} rejected")]);
        let told = plant
            .notifier
            .to("alice")
            .into_iter()
            .find(|m| m.message == Kind::CommandRejected)
            .expect("the author hears it");
        assert_eq!(
            told.reason.as_deref(),
            Some(format!("review {review} rejected by dana").as_str())
        );
        assert_eq!(
            plant
                .engine
                .status(&submitted.command_id)
                .await
                .expect("journaled")
                .status,
            State::Rejected
        );
    }

    #[tokio::test]
    async fn a_hold_freezes_the_partition_renews_and_releases() {
        let plant = plant(account(5000));
        let hold = plant
            .engine
            .hold("Account/881", "credit.limitReview", Some(600))
            .await
            .expect("held");
        assert_eq!(hold.ttl_seconds, 60, "capped at the aggregate's maximum");
        plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        let results = plant.engine.run_once().await.expect("ran");
        assert!(results.is_empty(), "blocked by the hold");
        assert!(
            plant
                .notifier
                .to("credit.limitReview")
                .iter()
                .any(|m| m.message == Kind::BlockedByHold)
        );
        plant.clock.set(1050);
        plant.engine.renew(&hold.id).await.expect("renewed");
        plant.clock.set(1100);
        assert!(
            plant.engine.run_once().await.expect("ran").is_empty(),
            "still held"
        );
        plant.engine.release(&hold.id).await.expect("released");
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Applied);
        assert!(matches!(
            plant.engine.renew(&hold.id).await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn a_cancelled_command_is_journaled_and_nobody_else_may_cancel_it() {
        let plant = plant(account(5000));
        let alice = plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        assert!(matches!(
            plant.engine.cancel(&alice.command_id, "bob").await,
            Err(Error::Forbidden(_))
        ));
        let result = plant
            .engine
            .cancel(&alice.command_id, "alice")
            .await
            .expect("cancelled");
        assert_eq!(result.status, State::Cancelled);
        assert_eq!(
            plant
                .engine
                .status(&alice.command_id)
                .await
                .expect("journaled")
                .status,
            State::Cancelled
        );
        assert!(matches!(
            plant.engine.status("c_nope").await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn a_served_query_is_one_the_registry_holds() {
        let plant = plant(MemoryStore::default());
        let engine = Engine::new(plant.store.clone(), plant.notifier.clone(), plant.clock.clone()).with_registry(
            Registry::parse(r#"{"queries":[{"query":"Q","capability":"credit.calculateRisk","file":"q","sha256":"aa","fields":["Account.creditLimit"]}]}"#)
                .expect("C"),
        );
        engine.served("credit.calculateRisk", "aa").expect("served");
        assert!(matches!(
            engine.served("credit.calculateRisk", "bb"),
            Err(Error::UnregisteredQuery(_))
        ));
        assert_eq!(engine.data_version().await.expect("read"), 0);
    }

    #[tokio::test]
    async fn the_engine_reloads_what_the_store_kept() {
        let store = Arc::new(account(5000));
        let notifier = Arc::new(MemoryNotifier::default());
        let clock = Arc::new(FixedClock::at(1000));
        let first = Engine::new(store.clone(), notifier.clone(), clock.clone())
            .with_aggregate(Aggregate::parse(ACCOUNT).expect("I"))
            .with_data_capability(DataCapability::parse(LIMIT).expect("F"));
        let submitted = first
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        let second = Engine::new(store.clone(), notifier, clock)
            .with_aggregate(Aggregate::parse(ACCOUNT).expect("I"))
            .with_data_capability(DataCapability::parse(LIMIT).expect("F"));
        second.load().await.expect("loaded");
        assert_eq!(
            second
                .status(&submitted.command_id)
                .await
                .expect("pending")
                .status,
            State::Queued
        );
        let results = second.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Applied);
    }

    #[tokio::test]
    async fn a_hold_elsewhere_blocks_nothing_and_the_plan_lists_what_is_behind() {
        let plant = plant(account(5000));
        plant
            .engine
            .hold("Account/999", "credit.other", Some(30))
            .await
            .expect("held elsewhere");
        let first = plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        let second = plant
            .engine
            .submit(Submission {
                data_capability: "credit.setStatus".to_string(),
                payload: BTreeMap::from([
                    ("accountId".to_string(), Value::from("881")),
                    ("tags".to_string(), Value::from("vip")),
                ]),
                target_id: None,
                based_on: None,
                idempotency_key: None,
                caller: Engine::caller("bob", "credit.Form", &[], &[]),
            })
            .await
            .expect("queued behind");
        let third = plant
            .engine
            .submit(limit("carol", 7000, 5000))
            .await
            .expect("queued, asked");
        assert_eq!(third.result.status, State::AwaitingConfirmation);
        let elsewhere = plant
            .engine
            .submit(Submission {
                target_id: Some("882".to_string()),
                ..limit("dave", 1, 1)
            })
            .await
            .expect("queued on another partition");
        assert_eq!(elsewhere.partition, "Account/882");
        plant
            .engine
            .cancel(&second.command_id, "bob")
            .await
            .expect("withdrawn");
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(
            results[0].status,
            State::Applied,
            "a hold on another partition blocks nothing"
        );
        let plan = plant.engine.plan(&first.command_id).expect("planned");
        assert_eq!(
            plan.dependencies.pending_commands,
            std::slice::from_ref(&third.command_id),
            "still pending behind it on its own partition: not the one withdrawn, not another partition's, not itself"
        );
        assert!(plan.dependencies.holds.is_empty(), "expected nothing");
        assert_eq!(
            plan.decision,
            Decision::Auto,
            "nothing left to confirm at the head"
        );
        let other = plant
            .engine
            .plan(&elsewhere.command_id)
            .expect("planned too");
        assert!(
            other.dependencies.pending_commands.is_empty(),
            "expected nothing"
        );
    }

    #[tokio::test]
    async fn an_expired_hold_leaves_the_store_and_a_live_one_stays() {
        let plant = plant(account(5000));
        let hold = plant
            .engine
            .hold("Account/881", "credit.limitReview", Some(30))
            .await
            .expect("held");
        plant.engine.run_once().await.expect("ran");
        let kept = plant.store.holds().await.expect("read");
        assert_eq!(kept.len(), 1, "alive: the store keeps it");
        assert_eq!(kept[0].id, hold.id);
        plant.clock.set(1000 + 31);
        plant.engine.run_once().await.expect("ran");
        assert!(
            plant.store.holds().await.expect("read").is_empty(),
            "expired: gone from the store"
        );
        plant
            .engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        let results = plant.engine.run_once().await.expect("ran");
        assert_eq!(
            results[0].status,
            State::Applied,
            "nothing freezes the partition any more"
        );
    }

    #[tokio::test]
    async fn an_addition_that_collides_under_an_aggregate_is_routed_to_the_resolver_and_told() {
        let store = MemoryStore::default().colliding(
            "credit.addPayment",
            Collision {
                unique_key: Some("Payment.reference".to_string()),
                ..Collision::default()
            },
        );
        let plant = plant(store);
        let engine = Engine::new(
            plant.store.clone(),
            plant.notifier.clone(),
            plant.clock.clone(),
        )
        .with_aggregate(Aggregate::parse(PAYMENT).expect("I"))
        .with_data_capability(DataCapability::parse(ADD).expect("F"));
        engine
            .submit(Submission {
                data_capability: "credit.addPayment".to_string(),
                payload: BTreeMap::from([("amount".to_string(), Value::from(12))]),
                target_id: Some("p1".to_string()),
                based_on: None,
                idempotency_key: Some("k".to_string()),
                caller: Engine::caller("alice", "credit.Form", &[], &[]),
            })
            .await
            .expect("queued");
        let results = engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Applied, "planned, not failed");
        let told = plant
            .notifier
            .to("alice")
            .into_iter()
            .find(|m| m.message == Kind::InvariantAtRisk)
            .expect("the author hears the reclassification");
        assert!(
            told.reason
                .as_deref()
                .is_some_and(|r| r.contains("found not to be one")),
            "{told:?}"
        );
        assert!(engine.plan("c_1").is_some(), "the Resolver planned it");
    }

    #[tokio::test]
    async fn a_relation_under_review_opens_a_review_at_the_head_and_asks_once() {
        let plant = plant(
            account(5000)
                .pointed_at("Account/881", vec![("Invoice.accountId".to_string(), 3)])
                .pointed_at("Account/882", vec![("Invoice.accountId".to_string(), 1)]),
        );
        let engine = Engine::new(
            plant.store.clone(),
            plant.notifier.clone(),
            plant.clock.clone(),
        )
        .with_aggregate(Aggregate::parse(REVIEWED).expect("I"))
        .with_data_capability(DataCapability::parse(CLOSE).expect("F"));
        let close = Submission {
            data_capability: "credit.closeAccount".to_string(),
            payload: BTreeMap::from([("accountId".to_string(), Value::from("881"))]),
            target_id: None,
            based_on: None,
            idempotency_key: None,
            caller: Engine::caller("alice", "credit.Form", &[], &[]),
        };
        let submitted = engine
            .submit(close)
            .await
            .expect("queued, nothing critical");
        assert_eq!(submitted.result.status, State::Queued);
        let results = engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::AwaitingReview);
        let review_id = results[0]
            .review_id
            .clone()
            .expect("the result names the review opened");
        assert_eq!(
            engine
                .status(&submitted.command_id)
                .await
                .expect("waiting")
                .review_id,
            Some(review_id.clone())
        );
        let asked = plant
            .notifier
            .to("role:approver")
            .into_iter()
            .find(|m| m.message == Kind::ReviewRequired)
            .expect("the default approver is told");
        assert_eq!(asked.reason.as_deref(), Some("an invariant wants a review"));
        assert!(engine.run_once().await.expect("ran").is_empty(), "waits");
        engine
            .decide(&review_id, "dana", &["role:approver".to_string()], true)
            .await
            .expect("approved");
        let results = engine.run_once().await.expect("ran");
        assert_eq!(
            results[0].status,
            State::Applied,
            "approved once, not asked again"
        );
        assert!(plant.store.is_tombstoned("Account/881"));
        // Another account's closure is asked on its own: an approval is
        // one command's, not everyone's.
        let other = Submission {
            payload: BTreeMap::from([("accountId".to_string(), Value::from("882"))]),
            ..close_882()
        };
        engine.submit(other).await.expect("queued");
        let results = engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::AwaitingReview, "asked on its own");
    }

    fn close_882() -> Submission {
        Submission {
            data_capability: "credit.closeAccount".to_string(),
            payload: BTreeMap::new(),
            target_id: None,
            based_on: None,
            idempotency_key: None,
            caller: Engine::caller("alice", "credit.Form", &[], &[]),
        }
    }

    #[tokio::test]
    async fn a_product_s_coded_invariant_adds_to_the_declared_rules() {
        let plant = plant(account(5000));
        let engine = Engine::new(
            plant.store.clone(),
            plant.notifier.clone(),
            plant.clock.clone(),
        )
        .with_aggregate(
            Aggregate::parse(
                r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"normal"}},"invariants":[{"id":"limit-under-ceiling","rule":"creditLimit <= 10000","onViolation":"reject"},{"id":"risk-fresh","rule":"a raise past 7000 wants a risk score younger than it","onViolation":"human_review"}]}"#,
            )
            .expect("I"),
        )
        .with_data_capability(DataCapability::parse(LIMIT).expect("F"))
        .with_plugins(plugins(Plugin::new().judge("Account", RiskFresh)));
        let modest = engine
            .submit(limit("alice", 6000, 5000))
            .await
            .expect("queued");
        assert_eq!(modest.result.status, State::Queued);
        let raise = engine
            .submit(limit("bob", 8000, 5000))
            .await
            .expect("queued for review");
        assert_eq!(
            raise.result.status,
            State::AwaitingReview,
            "the coded rule asks a human"
        );
        assert!(
            raise.warnings.iter().any(|w| w.contains("risk-fresh")),
            "{:?}",
            raise.warnings
        );
    }

    #[tokio::test]
    async fn a_store_that_is_down_is_an_error_not_a_lie() {
        let engine = Engine::new(
            Arc::new(crate::memory::BrokenStore),
            Arc::new(MemoryNotifier::default()),
            Arc::new(FixedClock::at(0)),
        )
        .with_data_capability(DataCapability::parse(LIMIT).expect("F"));
        assert!(matches!(engine.load().await, Err(Error::Store(_))));
        assert!(matches!(
            engine.submit(limit("alice", 1, 1)).await,
            Err(Error::Store(_))
        ));
    }

    /// A `Prepare` written as a closure.
    struct By<F>(F);

    #[async_trait::async_trait]
    impl<F> Prepare for By<F>
    where
        F: Fn(
                &Context<'_>,
                &BTreeMap<String, Value>,
                Option<&str>,
            ) -> std::result::Result<Prepared, Refusal>
            + Send
            + Sync,
    {
        async fn prepare(
            &self,
            context: &Context<'_>,
            payload: &BTreeMap<String, Value>,
            target_id: Option<&str>,
        ) -> std::result::Result<Prepared, Refusal> {
            (self.0)(context, payload, target_id)
        }
    }

    /// What every judge or projection of a test saw: a partition each time.
    #[derive(Clone, Default)]
    struct Seen(Arc<Mutex<Vec<String>>>);

    impl Seen {
        fn push(&self, what: String) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(what);
        }

        fn all(&self) -> Vec<String> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    impl Judge for Seen {
        fn violated(&self, _state: &BTreeMap<String, Value>, entry: &Entry) -> Vec<String> {
            self.push(entry.partition.clone());
            Vec::new()
        }
    }

    impl Project for Seen {
        fn observe(&self, write: &Write) {
            self.push(write.partition.clone());
        }
    }

    /// A value the units share.
    struct Model(&'static str);

    /// Ids that are always the same one.
    struct Fixed;

    impl Ids for Fixed {
        fn fresh(&self) -> String {
            "p-42".to_string()
        }
    }

    fn plugins(plugin: Plugin) -> Plugins {
        std::iter::once(plugin).collect()
    }

    fn board_with(row: BTreeMap<String, Value>, plugin: Plugin) -> Plant {
        let plant = board_plant(row);
        Plant {
            engine: plant.engine.with_plugins(plugins(plugin)),
            ..plant
        }
    }

    #[tokio::test]
    // One scenario: what the Prepare sees, then what it changes.
    #[allow(clippy::too_many_lines)]
    async fn a_prepare_rewrites_the_payload_and_the_target_on_the_projected_base() {
        let seen = Seen::default();
        let saw = seen.clone();
        let rename = By(
            move |context: &Context<'_>,
                  payload: &BTreeMap<String, Value>,
                  target: Option<&str>| {
                saw.push(format!(
                    "{} {} {} {:?} {}",
                    context.manifest.data_capability,
                    context.caller.by,
                    context.now,
                    target,
                    Value::from(
                        context
                            .base
                            .clone()
                            .into_iter()
                            .collect::<serde_json::Map<_, _>>()
                    )
                ));
                let model = context.shared.get::<Model>().map_or("none", |m| m.0);
                let note = payload.get("note").and_then(Value::as_str).unwrap_or("");
                Ok(Prepared {
                    payload: BTreeMap::from([
                        ("boardId".to_string(), Value::from("2")),
                        ("note".to_string(), Value::from(format!("{model}:{note}"))),
                        ("colour".to_string(), Value::from("red")),
                    ]),
                    target_id: Some("2".to_string()),
                    warnings: vec!["renamed".to_string()],
                    ..Prepared::default()
                })
            },
        );
        let plant = board_with(
            BTreeMap::from([("tags".to_string(), serde_json::json!(["a"]))]),
            Plugin::new()
                .prepare("board.rename", rename)
                .needs::<Model>(),
        );
        let mut shared = Shared::default();
        shared.insert(Arc::new(Model("m1")));
        let plant = Plant {
            engine: plant.engine.with_shared(shared),
            ..plant
        };
        plant.engine.load().await.expect("its need is shared");
        plant
            .engine
            .submit(on_board(
                "tag",
                "alice",
                "tags",
                serde_json::json!(["b"]),
                Value::Null,
            ))
            .await
            .expect("queued ahead");
        let submitted = plant
            .engine
            .submit(on_board(
                "rename",
                "bob",
                "note",
                Value::from("n"),
                Value::Null,
            ))
            .await
            .expect("prepared and queued");
        assert_eq!(
            seen.all(),
            [r#"board.rename bob 1000 Some("1") {"boardId":"1","tags":["a","b"]}"#],
            "the row as alice's pending tag leaves it, sets unioned"
        );
        assert_eq!(submitted.partition, "Board/2", "the target rewritten");
        assert_eq!(
            submitted.warnings,
            ["renamed"],
            "the Prepare's warnings; the keys it wrote are its own, never undeclared"
        );
        plant.engine.run_once().await.expect("ran");
        let row = plant.store.row_now("Board/2");
        assert_eq!(row["note"], Value::from("m1:n"), "the payload rewritten");
        assert_eq!(row["colour"], Value::from("red"));
        assert!(
            !plant.store.row_now("Board/1").contains_key("note"),
            "nothing written where the submission pointed"
        );
        // A command with no Prepare goes as it came.
        let tagged = plant
            .engine
            .submit(on_board(
                "note",
                "carol",
                "note",
                Value::from("plain"),
                Value::Null,
            ))
            .await
            .expect("queued");
        assert_eq!(tagged.partition, "Board/1");
        assert_eq!(seen.all().len(), 1, "the rename's Prepare alone ran once");
    }

    #[tokio::test]
    async fn a_prepare_refuses_with_its_kind_and_nothing_is_queued() {
        let refuse = By(
            |_: &Context<'_>, payload: &BTreeMap<String, Value>, _: Option<&str>| {
                if payload.get("note") == Some(&Value::from("ghost")) {
                    Err(Refusal::not_found("no such board"))
                } else {
                    Err(Refusal::unprocessable("the board is archived"))
                }
            },
        );
        let plant = board_with(BTreeMap::new(), Plugin::new().prepare("board.note", refuse));
        let ghost = plant
            .engine
            .submit(on_board(
                "note",
                "alice",
                "note",
                Value::from("ghost"),
                Value::Null,
            ))
            .await;
        assert_eq!(
            ghost,
            Err(Error::Refused(Refusal::not_found("no such board")))
        );
        let archived = plant
            .engine
            .submit(on_board(
                "note",
                "alice",
                "note",
                Value::from("x"),
                Value::Null,
            ))
            .await;
        assert!(
            matches!(&archived, Err(Error::Refused(r)) if r.kind == RefusalKind::Unprocessable),
            "{archived:?}"
        );
        assert_eq!(
            archived.map(|_| ()).unwrap_err().to_string(),
            "the board is archived"
        );
        assert!(
            plant.engine.run_once().await.expect("ran").is_empty(),
            "nothing ran"
        );
        assert!(
            plant.store.pending().await.expect("read").is_empty(),
            "nothing queued"
        );
    }

    #[tokio::test]
    async fn the_prepare_runs_after_authorisation_and_never_on_a_replay() {
        let calls = Seen::default();
        let counted = calls.clone();
        let count = By(
            move |_: &Context<'_>, payload: &BTreeMap<String, Value>, target: Option<&str>| {
                counted.push(String::new());
                Ok(Prepared {
                    payload: payload.clone(),
                    target_id: target.map(ToString::to_string),
                    warnings: Vec::new(),
                    ..Prepared::default()
                })
            },
        );
        let plant = plant(account(5000));
        let engine = plant.engine.with_plugins(plugins(
            Plugin::new().prepare("credit.requestLimitChange", count),
        ));
        let elsewhere = Submission {
            caller: Engine::caller("x", "orders.Form", &["credit.limit.write"], &[]),
            ..limit("x", 1, 5000)
        };
        assert!(matches!(
            engine.submit(elsewhere).await,
            Err(Error::Forbidden(_))
        ));
        assert!(calls.all().is_empty(), "refused before the Prepare");
        let keyed = Submission {
            idempotency_key: Some("k".to_string()),
            ..limit("alice", 6000, 5000)
        };
        let first = engine.submit(keyed.clone()).await.expect("queued");
        let again = engine.submit(keyed).await.expect("replayed");
        assert!(again.replayed);
        assert_eq!(again.command_id, first.command_id);
        assert_eq!(calls.all().len(), 1, "a replay prepares nothing");
    }

    #[tokio::test]
    async fn a_row_named_by_nobody_takes_a_fresh_id_and_its_prepare_sees_no_base() {
        let bases = Seen::default();
        let saw = bases.clone();
        let add = By(
            move |context: &Context<'_>,
                  payload: &BTreeMap<String, Value>,
                  target: Option<&str>| {
                saw.push(format!("{target:?} {}", context.base.len()));
                Ok(Prepared {
                    payload: payload.clone(),
                    target_id: None,
                    warnings: Vec::new(),
                    ..Prepared::default()
                })
            },
        );
        let plant = plant(MemoryStore::default());
        let engine = plant
            .engine
            .with_ids(Arc::new(Fixed))
            .with_plugins(plugins(Plugin::new().prepare("credit.addPayment", add)));
        let submitted = engine
            .submit(Submission {
                data_capability: "credit.addPayment".to_string(),
                payload: BTreeMap::from([("amount".to_string(), Value::from(12))]),
                target_id: None,
                based_on: None,
                idempotency_key: Some("k".to_string()),
                caller: Engine::caller("alice", "credit.Form", &[], &[]),
            })
            .await
            .expect("queued");
        assert_eq!(submitted.partition, "Payment/p-42", "the engine's ids");
        assert_eq!(bases.all(), ["None 0"]);
        // A Prepare that names no target leaves it to the payload.
        let keep = By(
            |_: &Context<'_>, payload: &BTreeMap<String, Value>, _: Option<&str>| {
                Ok(Prepared {
                    payload: payload.clone(),
                    target_id: None,
                    warnings: Vec::new(),
                    ..Prepared::default()
                })
            },
        );
        let engine = Engine::new(plant.store.clone(), plant.notifier.clone(), plant.clock)
            .with_aggregate(Aggregate::parse(ACCOUNT).expect("I"))
            .with_data_capability(DataCapability::parse(LIMIT).expect("F"))
            .with_plugins(plugins(
                Plugin::new().prepare("credit.requestLimitChange", keep),
            ));
        let named = engine
            .submit(limit("alice", 6000, 5000))
            .await
            .expect("queued");
        assert_eq!(named.partition, "Account/881");
    }

    #[tokio::test]
    async fn each_judge_sees_its_own_aggregate_only() {
        let accounts = Seen::default();
        let boards = Seen::default();
        let plant = board_plant(BTreeMap::new());
        let engine = plant
            .engine
            .with_aggregate(Aggregate::parse(ACCOUNT).expect("I"))
            .with_data_capability(DataCapability::parse(LIMIT).expect("F"))
            .with_plugins(
                [
                    Plugin::new().judge("Account", accounts.clone()),
                    Plugin::new().judge("Board", boards.clone()),
                ]
                .into_iter()
                .collect(),
            );
        engine.load().await.expect("both aggregates are known");
        engine
            .submit(limit("alice", 6000, 5000))
            .await
            .expect("queued");
        assert_eq!(accounts.all(), ["Account/881"]);
        assert!(boards.all().is_empty(), "blind to Account");
        engine
            .submit(on_board(
                "note",
                "bob",
                "note",
                Value::from("n"),
                Value::Null,
            ))
            .await
            .expect("queued");
        assert_eq!(boards.all(), ["Board/1"]);
        assert_eq!(accounts.all().len(), 1, "blind to Board");
    }

    #[tokio::test]
    async fn a_projection_observes_each_applied_write_once_and_no_rejected_one() {
        let accounts = Seen::default();
        let boards = Seen::default();
        let plant = plant(account(5000));
        let engine = plant
            .engine
            .with_aggregate(Aggregate::parse(BOARD).expect("I"))
            .with_plugins(
                [
                    Plugin::new().project("Account", accounts.clone()),
                    Plugin::new().project("Board", boards.clone()),
                ]
                .into_iter()
                .collect(),
            );
        engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        assert!(accounts.all().is_empty(), "nothing applied yet");
        engine.run_once().await.expect("applied");
        assert_eq!(accounts.all(), ["Account/881"]);
        engine
            .submit(limit("bob", 20_000, 8000))
            .await
            .expect("queued, warned");
        let results = engine.run_once().await.expect("ran");
        assert_eq!(results[0].status, State::Rejected);
        assert_eq!(
            accounts.all(),
            ["Account/881"],
            "a rejection writes nothing"
        );
        assert!(boards.all().is_empty(), "blind to Account");
    }

    #[tokio::test]
    async fn load_refuses_a_plugin_for_a_unit_it_does_not_register() {
        let noop = || {
            By(
                |_: &Context<'_>, payload: &BTreeMap<String, Value>, _: Option<&str>| {
                    Ok(Prepared {
                        payload: payload.clone(),
                        ..Prepared::default()
                    })
                },
            )
        };
        let loaded =
            |plugin: Plugin| async { board_with(BTreeMap::new(), plugin).engine.load().await };
        assert_eq!(
            loaded(
                Plugin::new()
                    .prepare("board.note", noop())
                    .judge("Board", Seen::default())
                    .project("Board", Seen::default())
            )
            .await,
            Ok(()),
            "every unit known"
        );
        for (plugin, why) in [
            (
                Plugin::new().prepare("board.nothing", noop()),
                "a Prepare for board.nothing",
            ),
            (
                Plugin::new()
                    .prepare("board.note", noop())
                    .prepare("board.note", noop()),
                "two Prepares for board.note",
            ),
            (
                Plugin::new().judge("Ghost", Seen::default()),
                "a Judge for Ghost",
            ),
            (
                Plugin::new().project("Ghost", Seen::default()),
                "a Project for Ghost",
            ),
        ] {
            let refused = loaded(plugin).await;
            assert!(
                matches!(&refused, Err(Error::Misconfigured(m)) if m.starts_with(why)),
                "{why}: {refused:?}"
            );
        }
    }

    #[test]
    fn a_rejection_on_the_real_state_names_its_invariants_or_the_resolver() {
        assert_eq!(rejection(&Verdict::default()), "the Resolver rejected it");
        assert_eq!(
            rejection(&Verdict {
                rejecting: vec!["limit-under-ceiling".to_string(), "risk-fresh".to_string()],
                reviewing: Vec::new(),
            }),
            "invariant(s) violated on the real state: limit-under-ceiling, risk-fresh"
        );
    }

    #[tokio::test]
    async fn a_write_the_store_refuses_is_rejected_with_its_reason_and_the_step_goes_on() {
        let seen = Seen::default();
        let plant = plant(account(5000).refusing("Account/881", "the ledger is closed"));
        let engine = plant
            .engine
            .with_plugins(plugins(Plugin::new().project("Account", seen.clone())));
        let queued = engine
            .submit(limit("alice", 8000, 5000))
            .await
            .expect("queued");
        let results = engine.run_once().await.expect("the step goes on");
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert_eq!(result.command_id, queued.command_id);
        assert_eq!(result.status, State::Rejected);
        assert_eq!(result.data_version, None);
        assert!(result.violations.is_empty(), "no invariant was violated");
        assert_eq!(
            engine
                .status(&queued.command_id)
                .await
                .expect("known")
                .status,
            State::Rejected
        );
        assert_eq!(
            plant.store.row_now("Account/881")["creditLimit"],
            Value::from(5000),
            "nothing written"
        );
        assert_eq!(engine.data_version().await.expect("read"), 0);
        assert!(seen.all().is_empty(), "a refused write is never observed");
        let journal = plant.store.journal();
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0].status, State::Rejected);
        let told = plant.notifier.to("alice");
        let rejected: Vec<&Message> = told
            .iter()
            .filter(|m| m.message == Kind::CommandRejected)
            .collect();
        assert_eq!(rejected.len(), 1, "{told:?}");
        assert_eq!(rejected[0].reason.as_deref(), Some("the ledger is closed"));
        assert!(
            engine.run_once().await.expect("ran").is_empty(),
            "a rejected head leaves the partition"
        );
    }

    /// A projection in the store's transaction that projects nothing.
    #[cfg(feature = "postgres")]
    struct Quiet;

    #[cfg(feature = "postgres")]
    #[async_trait::async_trait]
    impl crate::plugins::ProjectPg for Quiet {
        async fn project(
            &self,
            _write: &Write,
            _tx: &tokio_postgres::Transaction<'_>,
        ) -> std::result::Result<(), crate::plugins::ProjectionError> {
            Ok(())
        }
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn load_refuses_a_stored_projection_for_an_aggregate_it_does_not_register() {
        let loaded =
            |plugin: Plugin| async { board_with(BTreeMap::new(), plugin).engine.load().await };
        assert_eq!(
            loaded(Plugin::new().project_pg("Board", Quiet)).await,
            Ok(())
        );
        let refused = loaded(Plugin::new().project_pg("Ghost", Quiet)).await;
        assert!(
            matches!(&refused, Err(Error::Misconfigured(m)) if m.starts_with("a ProjectPg for Ghost")),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn load_refuses_a_need_nothing_shares_and_accepts_it_shared() {
        let needing = || Plugin::new().needs::<Model>();
        let missing = board_with(BTreeMap::new(), needing()).engine.load().await;
        assert!(
            matches!(&missing, Err(Error::Misconfigured(m)) if m.contains("Model") && m.contains("nothing shares")),
            "{missing:?}"
        );
        let mut shared = Shared::default();
        shared.insert(Arc::new(Model("m")));
        let by_the_server = board_with(BTreeMap::new(), needing())
            .engine
            .with_shared(shared)
            .load()
            .await;
        assert_eq!(by_the_server, Ok(()));
        let by_a_plugin = board_plant(BTreeMap::new())
            .engine
            .with_plugins(
                [needing(), Plugin::new().share(Arc::new(Model("p")))]
                    .into_iter()
                    .collect(),
            )
            .load()
            .await;
        assert_eq!(by_a_plugin, Ok(()), "another plugin shares it");
    }

    #[tokio::test]
    async fn undeclared_keys_are_the_submitted_ones_never_those_a_prepare_writes() {
        let retitle = By(
            |_: &Context<'_>, payload: &BTreeMap<String, Value>, target: Option<&str>| {
                let mut written = payload.clone();
                if let Some(title) = written.remove("title") {
                    written.insert("note".to_string(), title);
                }
                Ok(Prepared {
                    payload: written,
                    target_id: target.map(ToString::to_string),
                    ..Prepared::default()
                })
            },
        );
        let plant = board_plant(BTreeMap::new());
        let engine = plant
            .engine
            .with_data_capability(board_capability(
                "retitle",
                "note",
                r#","payload":{"boardId":"string","title":"string"}"#,
            ))
            .with_plugins(plugins(Plugin::new().prepare("board.retitle", retitle)));
        let clean = engine
            .submit(on_board(
                "retitle",
                "alice",
                "title",
                Value::from("t"),
                Value::Null,
            ))
            .await
            .expect("queued");
        assert!(
            clean.warnings.is_empty(),
            "note is the Prepare's key, title the author's: {:?}",
            clean.warnings
        );
        let mut sent = on_board("retitle", "bob", "title", Value::from("u"), Value::Null);
        sent.payload
            .insert("colour".to_string(), Value::from("red"));
        let warned = engine.submit(sent).await.expect("queued");
        assert_eq!(
            warned.warnings,
            ["payload key(s) colour not declared by board.retitle"]
        );
        engine.run_once().await.expect("ran");
        assert_eq!(plant.store.row_now("Board/1")["note"], Value::from("u"));
    }

    /// Notes on the board the credit limit of the account its payload
    /// names, as the queue leaves it; refuses when there is no such
    /// account.
    struct Linked;

    #[async_trait::async_trait]
    impl Prepare for Linked {
        async fn prepare(
            &self,
            context: &Context<'_>,
            payload: &BTreeMap<String, Value>,
            target_id: Option<&str>,
        ) -> std::result::Result<Prepared, Refusal> {
            let account = payload
                .get("accountId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let row = context.row(&format!("Account/{account}")).await?;
            let limit = row
                .get("creditLimit")
                .ok_or_else(|| Refusal::not_found(format!("no account {account}")))?;
            Ok(Prepared {
                payload: BTreeMap::from([(
                    "note".to_string(),
                    Value::from(format!("limit {limit}")),
                )]),
                target_id: target_id.map(ToString::to_string),
                ..Prepared::default()
            })
        }
    }

    /// Accounts and boards, a board's note prepared by [`Linked`].
    fn linked_plant(store: MemoryStore) -> Plant {
        let store = Arc::new(store);
        let notifier = Arc::new(MemoryNotifier::default());
        let clock = Arc::new(FixedClock::at(1000));
        let engine = Engine::new(store.clone(), notifier.clone(), clock.clone())
            .with_aggregate(Aggregate::parse(ACCOUNT).expect("I"))
            .with_aggregate(Aggregate::parse(BOARD).expect("I"))
            .with_data_capability(DataCapability::parse(LIMIT).expect("F"))
            .with_data_capability(board_capability("note", "note", ""))
            .with_plugins(plugins(Plugin::new().prepare("board.note", Linked)));
        Plant {
            engine,
            store,
            notifier,
            clock,
        }
    }

    fn note_on(account: &str) -> Submission {
        let mut note = on_board("note", "bob", "note", Value::from("x"), Value::Null);
        note.payload
            .insert("accountId".to_string(), Value::from(account));
        note
    }

    #[tokio::test]
    async fn a_prepare_reads_another_partition_s_row_as_the_queue_leaves_it() {
        let plant = linked_plant(account(5000).with_row("Board/1", BTreeMap::new()));
        plant.engine.load().await.expect("fits");
        plant
            .engine
            .submit(limit("alice", 6000, 5000))
            .await
            .expect("queued, not applied");
        let noted = plant.engine.submit(note_on("881")).await.expect("prepared");
        assert_eq!(noted.partition, "Board/1");
        plant.engine.run_once().await.expect("ran");
        assert_eq!(
            plant.store.row_now("Board/1")["note"],
            Value::from("limit 6000"),
            "alice's raise, read while it was still pending"
        );
    }

    #[tokio::test]
    async fn a_prepare_refuses_when_the_row_it_references_is_missing() {
        let plant = linked_plant(account(5000).with_row("Board/1", BTreeMap::new()));
        let refused = plant.engine.submit(note_on("999")).await;
        assert_eq!(
            refused.map(|s| s.command_id),
            Err(Error::Refused(Refusal::not_found("no account 999")))
        );
        assert!(
            plant.store.pending().await.expect("read").is_empty(),
            "nothing queued"
        );
    }

    #[tokio::test]
    async fn a_store_failing_under_a_prepare_s_read_is_the_engine_s_answer() {
        const LOOSE: &str = r#"{"dataCapability":"board.note","owner":"board","version":1,"effect":"insert","target":{"aggregate":"Board"},"callableBy":["*"]}"#;
        let engine = Engine::new(
            Arc::new(crate::memory::BrokenStore),
            Arc::new(MemoryNotifier::default()),
            Arc::new(FixedClock::at(0)),
        )
        .with_data_capability(DataCapability::parse(LOOSE).expect("F"))
        .with_plugins(plugins(Plugin::new().prepare("board.note", Linked)));
        let failed = engine
            .submit(Submission {
                data_capability: "board.note".to_string(),
                payload: BTreeMap::from([("accountId".to_string(), Value::from("881"))]),
                caller: carol(),
                ..Submission::default()
            })
            .await;
        assert_eq!(
            failed.map(|s| s.command_id),
            Err(Error::Store("down".to_string())),
            "the store's error, not the refusal the Prepare made of it"
        );
    }

    /// One row to any query, keeping the queries asked.
    #[derive(Default)]
    struct Tables(Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl Rows for Tables {
        async fn rows(&self, sql: &str, _parameters: &[Value]) -> Result<Value> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(sql.to_string());
            Ok(serde_json::json!([{ "n": 1 }]))
        }
    }

    /// Warns of what it was lent: now, now in milliseconds, and the rows
    /// its SQL answers when it has SQL.
    struct Peek;

    #[async_trait::async_trait]
    impl Prepare for Peek {
        async fn prepare(
            &self,
            context: &Context<'_>,
            payload: &BTreeMap<String, Value>,
            target_id: Option<&str>,
        ) -> std::result::Result<Prepared, Refusal> {
            let rows = match context.rows {
                Some(rows) => rows
                    .rows("select n from t", &[])
                    .await
                    .map_err(|e| Refusal::unprocessable(e.to_string()))?,
                None => Value::Null,
            };
            Ok(Prepared {
                payload: payload.clone(),
                target_id: target_id.map(ToString::to_string),
                warnings: vec![format!("{} {} {rows}", context.now, context.now_millis)],
                ..Prepared::default()
            })
        }
    }

    #[tokio::test]
    async fn a_prepare_reads_sql_when_the_engine_has_some_and_stamps_milliseconds() {
        let plant = board_plant(BTreeMap::new());
        plant.clock.set_millis(1_000_250);
        let engine = plant
            .engine
            .with_plugins(plugins(Plugin::new().prepare("board.note", Peek)));
        let note = || on_board("note", "bob", "note", Value::from("n"), Value::Null);
        let over_memory = engine.submit(note()).await.expect("queued");
        assert_eq!(over_memory.warnings, ["1000 1000250 null"], "no SQL");
        let tables = Arc::new(Tables::default());
        let engine = engine.with_rows(tables.clone());
        let over_sql = engine.submit(note()).await.expect("queued");
        assert_eq!(over_sql.warnings, [r#"1000 1000250 [{"n":1}]"#]);
        assert_eq!(
            *tables
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            ["select n from t"]
        );
    }

    /// Writes as it came, then asks for these follow-ups, attaching this.
    struct Then(Vec<Submission>, Option<Value>);

    #[async_trait::async_trait]
    impl Prepare for Then {
        async fn prepare(
            &self,
            _context: &Context<'_>,
            payload: &BTreeMap<String, Value>,
            target_id: Option<&str>,
        ) -> std::result::Result<Prepared, Refusal> {
            Ok(Prepared {
                payload: payload.clone(),
                target_id: target_id.map(ToString::to_string),
                then: self.0.clone(),
                attachment: self.1.clone(),
                ..Prepared::default()
            })
        }
    }

    /// A board whose note is followed by `then`, and the accounts.
    fn followed(then: Vec<Submission>) -> Plant {
        let plant = linked_plant(account(5000).with_row("Board/1", BTreeMap::new()));
        let engine = Engine::new(
            plant.store.clone(),
            plant.notifier.clone(),
            plant.clock.clone(),
        )
        .with_aggregate(Aggregate::parse(ACCOUNT).expect("I"))
        .with_aggregate(Aggregate::parse(BOARD).expect("I"))
        .with_data_capability(DataCapability::parse(LIMIT).expect("F"))
        .with_data_capability(board_capability("note", "note", ""))
        .with_data_capability(board_capability("tag", "tags", ""))
        .with_plugins(plugins(
            Plugin::new().prepare("board.note", Then(then, None)),
        ));
        Plant { engine, ..plant }
    }

    #[tokio::test]
    async fn follow_ups_are_submitted_in_order_by_the_same_caller_once_the_first_is_queued() {
        let raise = Submission {
            caller: Caller::default(),
            ..limit("mallory", 6000, 5000)
        };
        let tag = on_board(
            "tag",
            "mallory",
            "tags",
            serde_json::json!(["z"]),
            Value::Null,
        );
        let plant = followed(vec![raise, tag]);
        plant.engine.load().await.expect("fits");
        let submitted = plant
            .engine
            .submit(on_board(
                "note",
                "bob",
                "note",
                Value::from("n"),
                Value::Null,
            ))
            .await
            .expect("queued");
        assert_eq!(submitted.command_id, "c_1");
        let then: Vec<(String, String)> = submitted
            .then
            .iter()
            .map(|follow| match follow {
                FollowUp::Submitted(one) => (one.command_id.clone(), one.partition.clone()),
                FollowUp::Refused { error, .. } => (String::new(), error.clone()),
            })
            .collect();
        assert_eq!(
            then,
            [
                ("c_2".to_string(), "Account/881".to_string()),
                ("c_3".to_string(), "Board/1".to_string())
            ],
            "after the first, in order, each through the whole path"
        );
        let authors: Vec<String> = plant
            .store
            .pending()
            .await
            .expect("read")
            .into_iter()
            .map(|e| e.by)
            .collect();
        assert_eq!(authors, ["bob", "bob", "bob"], "the first's caller");
        let applied = plant.engine.run_once().await.expect("ran");
        assert!(
            applied.iter().all(|r| r.status == State::Applied),
            "{applied:?}"
        );
        assert_eq!(
            plant.store.row_now("Account/881")["creditLimit"],
            Value::from(6000)
        );
        assert_eq!(
            plant.store.row_now("Board/1")["tags"],
            serde_json::json!(["z"])
        );
    }

    #[tokio::test]
    async fn a_refused_follow_up_undoes_nothing_and_the_ones_after_it_are_not_submitted() {
        let tag = || on_board("tag", "bob", "tags", serde_json::json!(["z"]), Value::Null);
        let unknown = Submission {
            data_capability: "board.nothing".to_string(),
            ..Submission::default()
        };
        let plant = followed(vec![unknown, tag()]);
        let submitted = plant
            .engine
            .submit(on_board(
                "note",
                "bob",
                "note",
                Value::from("n"),
                Value::Null,
            ))
            .await
            .expect("the first is queued");
        assert_eq!(submitted.result.status, State::Queued);
        assert_eq!(
            submitted.then,
            [FollowUp::Refused {
                data_capability: "board.nothing".to_string(),
                error: "no DataCapability named board.nothing".to_string(),
            }]
        );
        assert_eq!(
            plant.store.pending().await.expect("read").len(),
            1,
            "the first alone, the tag after the refusal never submitted"
        );
        // A follow-up is authorised as any submission: a board's caller may
        // not raise a limit.
        let plant = followed(vec![limit("bob", 6000, 5000), tag()]);
        let boarded = Submission {
            caller: Engine::caller("bob", "board.Form", &[], &[]),
            ..on_board("note", "bob", "note", Value::from("n"), Value::Null)
        };
        let submitted = plant.engine.submit(boarded).await.expect("queued");
        assert!(
            matches!(
                submitted.then.as_slice(),
                [FollowUp::Refused { data_capability, .. }] if data_capability == "credit.requestLimitChange"
            ),
            "{:?}",
            submitted.then
        );
    }

    #[tokio::test]
    async fn a_follow_up_asking_for_follow_ups_is_refused_and_a_replay_submits_none() {
        let tag = || on_board("tag", "bob", "tags", serde_json::json!(["z"]), Value::Null);
        let plant = followed(vec![tag()]);
        let engine = plant.engine.with_plugins(plugins(Plugin::new().prepare(
            "board.tag",
            Then(
                vec![on_board(
                    "note",
                    "bob",
                    "note",
                    Value::from("m"),
                    Value::Null,
                )],
                None,
            ),
        )));
        let keyed = Submission {
            idempotency_key: Some("k".to_string()),
            ..on_board("note", "bob", "note", Value::from("n"), Value::Null)
        };
        let submitted = engine.submit(keyed.clone()).await.expect("queued");
        assert_eq!(
            submitted.then,
            [FollowUp::Refused {
                data_capability: "board.tag".to_string(),
                error: "misconfigured: board.tag runs as a follow-up: its Prepare may not ask for follow-ups of its own (1 asked)".to_string(),
            }]
        );
        assert_eq!(
            plant.store.pending().await.expect("read").len(),
            1,
            "the tag was not queued"
        );
        let again = engine.submit(keyed).await.expect("replayed");
        assert!(again.replayed);
        assert!(again.then.is_empty(), "a replay submits no follow-up");
        // A follow-up's own key replays it.
        let keyed_tag = Submission {
            idempotency_key: Some("t".to_string()),
            ..tag()
        };
        let plant = followed(vec![keyed_tag]);
        let note = || on_board("note", "bob", "note", Value::from("n"), Value::Null);
        plant.engine.submit(note()).await.expect("queued");
        let second = plant.engine.submit(note()).await.expect("queued");
        assert!(
            matches!(second.then.as_slice(), [FollowUp::Submitted(one)] if one.replayed),
            "{:?}",
            second.then
        );
    }

    #[tokio::test]
    async fn an_attachment_is_kept_with_the_command_and_answered_once_it_is_applied() {
        let archive = serde_json::json!({ "archive": "881.zip" });
        let plant = plant(account(5000));
        let engine = plant.engine.with_plugins(plugins(Plugin::new().prepare(
            "credit.requestLimitChange",
            Then(Vec::new(), Some(archive.clone())),
        )));
        let queued = engine
            .submit(limit("alice", 6000, 5000))
            .await
            .expect("queued");
        assert_eq!(queued.result.attachment, None, "not before it is applied");
        assert_eq!(
            engine
                .status(&queued.command_id)
                .await
                .expect("known")
                .attachment,
            None
        );
        let kept = plant.store.pending().await.expect("read");
        assert_eq!(kept[0].attachment, Some(archive.clone()), "kept with it");
        let applied = engine.run_once().await.expect("ran");
        assert_eq!(applied[0].status, State::Applied);
        assert_eq!(applied[0].attachment, Some(archive.clone()));
        assert_eq!(
            engine
                .status(&queued.command_id)
                .await
                .expect("journaled")
                .attachment,
            Some(archive.clone())
        );
        assert_eq!(plant.store.journal()[0].attachment, Some(archive.clone()));
        let over = engine
            .submit(limit("bob", 20_000, 6000))
            .await
            .expect("queued, at risk");
        let rejected = engine.run_once().await.expect("ran");
        assert_eq!(rejected[0].status, State::Rejected);
        assert_eq!(rejected[0].attachment, None, "a rejection attaches nothing");
        assert_eq!(
            engine
                .status(&over.command_id)
                .await
                .expect("journaled")
                .attachment,
            None
        );
    }
}
