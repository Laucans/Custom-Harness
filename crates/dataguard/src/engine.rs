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
//! The queue and the reviews live behind `std::sync::Mutex`: every lock is
//! taken and released between two awaits, never across one.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::aggregate::Aggregate;
use crate::api::{Submission, Submitted};
use crate::command::{BasedOn, CommandResult, Effect, Entry, Projection, State};
use crate::guard::{self, Caller, Classification, Invariants, NoInvariants, Verdict};
use crate::manifest::{DataCapability, Idempotency};
use crate::messages::{Action, Kind, Message, Notifier};
use crate::queue::{Hold, Queue, Step};
use crate::reconcile::{Base, Candidate, NoProposer, Proposer, Review, Status};
use crate::registry::Registry;
use crate::resolver::{self, Decision, ImpactPlan, Situation};
use crate::store::{Clock, Store, Write};
use crate::{Error, Result};

/// How many heads one partition may go through in one run: a parked head
/// is set aside and the next looked at, up to this many times.
const HEADS_PER_RUN: usize = 64;

/// The engine.
pub struct Engine {
    store: Arc<dyn Store>,
    notifier: Arc<dyn Notifier>,
    clock: Arc<dyn Clock>,
    invariants: Arc<dyn Invariants>,
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
            invariants: Arc::new(NoInvariants),
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

    /// The product's invariants, executed.
    #[must_use]
    pub fn with_invariants(mut self, invariants: Arc<dyn Invariants>) -> Self {
        self.invariants = invariants;
        self
    }

    /// The AI that proposes reconciliations.
    #[must_use]
    pub fn with_proposer(mut self, proposer: Arc<dyn Proposer>) -> Self {
        self.proposer = proposer;
        self
    }

    /// Loads what the store kept: the pending entries and the live holds.
    ///
    /// # Errors
    ///
    /// The store's.
    pub async fn load(&self) -> Result<()> {
        let pending = self.store.pending().await?;
        let holds = self.store.holds().await?;
        *self.lock_queue() = Queue::load(pending, holds);
        Ok(())
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

    /// A command enters the queue.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownDataCapability`], [`Error::Forbidden`] (the caller,
    /// a missing idempotency key), or the store's.
    pub async fn submit(&self, submission: Submission) -> Result<Submitted> {
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
            return self.replayed(&existing).await;
        }
        let target_id = submission
            .target_id
            .clone()
            .or_else(|| Self::target_from_payload(&manifest, &submission.payload))
            .unwrap_or_else(|| self.next_id("new"));
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
            payload: submission.payload.clone(),
            effect: manifest.effect,
            touches: manifest.touches.clone(),
            mode: manifest.mode,
            projection: Projection::default(),
            state: State::Queued,
            confirmation: None,
            parked: None,
            requeued_from: None,
            idempotency_key: submission.idempotency_key.clone(),
        };
        let Judged {
            mut warnings,
            mut messages,
            needs_review,
        } = self.judge(&manifest, &entry, &row, &submission.caller);
        let (entry, queued) = self.lock_queue().submit(entry, row, needs_review);
        messages.extend(queued);
        self.store.save(&entry).await?;
        self.notify_all(&messages);
        if entry.state == State::AwaitingConfirmation {
            warnings.push("a value is declared ahead: confirm or cancel".to_string());
        }
        Ok(Submitted {
            command_id: command,
            partition,
            replayed: false,
            warnings,
            result: self.result_of(&entry),
        })
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
        let projected = guard::projected(row, entry);
        let verdict = guard::check(aggregate, self.invariants.as_ref(), &projected, entry);
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
        let (fresh, messages) = self.lock_queue().requeue(command, row)?;
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
        let projected = guard::projected(&row, &entry);
        let aggregate = self.aggregates.get(&manifest.target.aggregate);
        let verdict = aggregate.map_or_else(Verdict::default, |aggregate| {
            guard::check(aggregate, self.invariants.as_ref(), &projected, &entry)
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
            Decision::Rejected => self.rejected(&entry, &verdict).await,
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

    /// Applies the head: the write, the version, the journal, the word.
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
            ..CommandResult::of(entry, None)
        };
        let version = self.store.apply(&write, &result).await?;
        result.data_version = Some(version);
        let messages = self.lock_queue().applied(&entry.command, version)?;
        let applied = self.entry(&entry.command)?;
        self.store.save(&applied).await?;
        self.notify_all(&messages);
        Ok(result)
    }

    /// Rejects the head on the real state.
    async fn rejected(&self, entry: &Entry, verdict: &Verdict) -> Result<CommandResult> {
        let reason = if verdict.rejecting.is_empty() {
            "the Resolver rejected it".to_string()
        } else {
            format!(
                "invariant(s) violated on the real state: {}",
                verdict.rejecting.join(", ")
            )
        };
        let messages = self
            .lock_queue()
            .rejected(&entry.command, &verdict.rejecting, &reason)?;
        let rejected = self.entry(&entry.command)?;
        self.store.save(&rejected).await?;
        let result = CommandResult {
            violations: verdict.rejecting.clone(),
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

    struct Ceiling;

    impl Invariants for Ceiling {
        fn violated(
            &self,
            _aggregate: &str,
            state: &BTreeMap<String, Value>,
            _entry: &Entry,
        ) -> Vec<String> {
            if state
                .get("creditLimit")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                > 10_000
            {
                vec!["limit-under-ceiling".to_string()]
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
            .with_data_capability(DataCapability::parse(STATUS).expect("F"))
            .with_invariants(Arc::new(Ceiling));
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
        // One run: Alice applied, Bob parked, Carol applied after.
        let results = plant.engine.run_once().await.expect("ran");
        let statuses: Vec<(String, State)> = results
            .iter()
            .map(|r| (r.command_id.clone(), r.status))
            .collect();
        assert_eq!(
            statuses,
            [
                (alice.command_id.clone(), State::Applied),
                (carol.command_id.clone(), State::Applied)
            ]
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
        plant.clock.set(1000 + crate::queue::PARKED_TTL_SECONDS);
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
}
