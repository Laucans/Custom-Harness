//! The `DataQueue` (contract G): one FIFO per partition, fixed positions, a
//! two-layer projected view, overwrite confirmations, parked commands,
//! holds, expiry — the rules of the architecture, pure.
//!
//! The engine loads the pending entries from the store, drives this model,
//! and saves what moved. Nothing here reads a clock: every rule that needs
//! the time takes `now`, seconds since the epoch.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::command::{BasedOn, Entry, Mode, Parked, PendingAhead, Projection, State};
use crate::messages::{Action, DeclaredAhead, HoldRef, Kind, Message};
use crate::{Error, Result};

/// How long a parked command waits before it expires: a human
/// confirmation takes hours, not seconds.
pub const PARKED_TTL_SECONDS: u64 = 24 * 3600;

/// How long before a parked command's expiry its author is warned.
pub const PARKED_WARNING_SECONDS: u64 = 3600;

/// How long before a hold's expiry its owner is warned.
pub const HOLD_WARNING_SECONDS: u64 = 15;

/// A workflow's freeze on a partition from a position on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    /// Its id.
    pub id: String,
    /// `<Aggregate>/<id>`.
    pub partition: String,
    /// Who holds.
    pub by: String,
    /// The position it froze the partition from: commands at or past it
    /// wait.
    pub position: u64,
    /// When it expires.
    pub expires: u64,
    /// Its `ttl`, for a renewal.
    pub ttl_seconds: u64,
    /// Whether its owner was warned of the expiry.
    #[serde(default)]
    pub warned: bool,
}

/// What the head of a partition is ready for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Nothing waits in the partition.
    Empty,
    /// The head may be applied: it is confirmed, or needed no confirmation.
    Ready(String),
    /// A hold freezes the partition at or before the head.
    Blocked(String),
    /// The head awaits a review; the partition waits with it.
    Waiting(String),
    /// The head reached the front unconfirmed: it was parked, and the
    /// queue went on — look again.
    Parked(String),
}

/// The queue: every pending entry, every live hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queue {
    entries: Vec<Entry>,
    holds: Vec<Hold>,
    next_position: BTreeMap<String, u64>,
}

impl Queue {
    /// A queue holding these pending entries and holds, as a store kept
    /// them.
    #[must_use]
    pub fn load(entries: Vec<Entry>, holds: Vec<Hold>) -> Self {
        let mut next_position: BTreeMap<String, u64> = BTreeMap::new();
        for entry in &entries {
            let next = next_position.entry(entry.partition.clone()).or_insert(0);
            *next = (*next).max(entry.position.saturating_add(1));
        }
        for hold in &holds {
            let next = next_position.entry(hold.partition.clone()).or_insert(0);
            *next = (*next).max(hold.position.saturating_add(1));
        }
        Self {
            entries,
            holds,
            next_position,
        }
    }

    /// Every entry, final ones included until the engine saves them off.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Every live hold.
    #[must_use]
    pub fn holds(&self) -> &[Hold] {
        &self.holds
    }

    /// The entry with this id.
    #[must_use]
    pub fn entry(&self, command: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.command == command)
    }

    fn entry_mut(&mut self, command: &str) -> Result<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|e| e.command == command)
            .ok_or_else(|| Error::NotFound(format!("command {command}")))
    }

    /// The next position of a partition, taken.
    fn take_position(&mut self, partition: &str) -> u64 {
        let next = self.next_position.entry(partition.to_string()).or_insert(0);
        let taken = *next;
        *next = next.saturating_add(1);
        taken
    }

    /// The non-final entries of a partition, by position.
    fn pending_in(&self, partition: &str) -> Vec<&Entry> {
        let mut found: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.partition == partition && !e.state.is_final())
            .collect();
        found.sort_by_key(|e| e.position);
        found
    }

    /// What is declared ahead of `position` in `partition` on `fields`:
    /// the pending commands before it that write one of them.
    fn ahead_of(&self, partition: &str, position: u64, fields: &[String]) -> Vec<PendingAhead> {
        self.pending_in(partition)
            .into_iter()
            .filter(|e| e.position < position)
            .filter(|e| e.fields().iter().any(|f| fields.contains(f)))
            .map(|e| PendingAhead {
                command: e.command.clone(),
                by: e.by.clone(),
                value: fields.iter().find_map(|f| e.payload.get(f).cloned()),
                state: e.state,
            })
            .collect()
    }

    /// Whether a command written on `based_on` is stale against the
    /// confirmed values of its fields: a field it read has moved since.
    fn moved_since(
        based_on: &BasedOn,
        confirmed: &BTreeMap<String, Value>,
        fields: &[String],
    ) -> bool {
        fields.iter().any(|f| {
            based_on
                .values
                .get(f)
                .is_some_and(|read| confirmed.get(f).is_some_and(|now| now != read))
        })
    }

    /// Enqueues `entry` at the end of its partition, with the projected
    /// view it sees: the confirmed row, and what is pending ahead. The
    /// entry comes with its payload, mode and `basedOn`; its position,
    /// projection and state are set here. What the author is told comes
    /// back with it.
    ///
    /// `confirmed` is the row as the store holds it now.
    pub fn submit(
        &mut self,
        mut entry: Entry,
        confirmed: BTreeMap<String, Value>,
        needs_review: bool,
    ) -> (Entry, Vec<Message>) {
        let fields = entry.fields();
        entry.position = self.take_position(&entry.partition);
        let pending_ahead = self.ahead_of(&entry.partition, entry.position, &fields);
        let stale_ahead = pending_ahead.first().cloned();
        let moved = Self::moved_since(&entry.based_on, &confirmed, &fields);
        let field = fields.first().cloned().unwrap_or_default();
        let applied_value = confirmed.get(&field).cloned().unwrap_or(Value::Null);
        entry.projection = Projection {
            confirmed,
            pending_ahead,
        };
        let mut messages = vec![Message::new(
            Kind::CommandQueued,
            &[&entry.by],
            &entry.command,
        )];
        entry.state = if needs_review {
            State::AwaitingReview
        } else {
            match entry.mode {
                Mode::ConfirmOnStale if stale_ahead.is_some() || moved => {
                    let declared = stale_ahead.as_ref().map_or_else(
                        || DeclaredAhead {
                            value: applied_value,
                            by: "(applied)".to_string(),
                            command: "(applied)".to_string(),
                        },
                        |ahead| DeclaredAhead {
                            value: ahead.value.clone().unwrap_or(Value::Null),
                            by: ahead.by.clone(),
                            command: ahead.command.clone(),
                        },
                    );
                    messages.push(Message {
                        field: Some(format!("{}.{field}", entry.partition)),
                        your_value: entry.payload.get(&field).cloned(),
                        declared_ahead: Some(declared),
                        actions: vec![Action::ConfirmOverwrite, Action::Cancel, Action::Edit],
                        ..Message::new(Kind::ValueDeclaredAhead, &[&entry.by], &entry.command)
                    });
                    State::AwaitingConfirmation
                }
                Mode::ConfirmOnStale | Mode::Relative | Mode::Overwrite => State::Queued,
            }
        };
        self.entries.push(entry.clone());
        (entry, messages)
    }

    /// The author accepts to overwrite what is declared ahead.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for an unknown command, [`Error::NotNow`] for one
    /// that awaits no confirmation.
    pub fn confirm(&mut self, command: &str, by: &str) -> Result<&Entry> {
        let entry = self.entry_mut(command)?;
        if entry.state != State::AwaitingConfirmation {
            return Err(Error::NotNow(format!(
                "{command} is {:?}, not awaiting a confirmation",
                entry.state
            )));
        }
        entry.state = State::Confirmed;
        entry.confirmation = Some(by.to_string());
        Ok(entry)
    }

    /// The Resolver sent the head to a human: it waits for the review in
    /// its place.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], or [`Error::NotNow`] for a command already
    /// final.
    pub fn await_review(&mut self, command: &str) -> Result<&Entry> {
        let entry = self.entry_mut(command)?;
        if entry.state.is_final() {
            return Err(Error::NotNow(format!("{command} is {:?}", entry.state)));
        }
        entry.state = State::AwaitingReview;
        Ok(entry)
    }

    /// A review ended: approved, the command goes on as confirmed;
    /// rejected, it is rejected.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], or [`Error::NotNow`] for a command that awaits
    /// no review.
    pub fn reviewed(&mut self, command: &str, approved: bool) -> Result<&Entry> {
        let entry = self.entry_mut(command)?;
        if entry.state != State::AwaitingReview {
            return Err(Error::NotNow(format!(
                "{command} is {:?}, not awaiting a review",
                entry.state
            )));
        }
        entry.state = if approved {
            State::Confirmed
        } else {
            State::Rejected
        };
        Ok(entry)
    }

    /// The author withdraws the command. What was behind it and saw it
    /// ahead is told.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], or [`Error::NotNow`] for a command already
    /// final.
    pub fn cancel(&mut self, command: &str) -> Result<Vec<Message>> {
        let entry = self.entry_mut(command)?;
        if entry.state.is_final() {
            return Err(Error::NotNow(format!(
                "{command} is already {:?}",
                entry.state
            )));
        }
        entry.state = State::Cancelled;
        let partition = entry.partition.clone();
        Ok(self.ahead_resolved(&partition, command))
    }

    /// `AheadResolved` to every author behind `command` in `partition` who
    /// saw it ahead.
    fn ahead_resolved(&self, partition: &str, command: &str) -> Vec<Message> {
        self.pending_in(partition)
            .into_iter()
            .filter(|e| {
                e.projection
                    .pending_ahead
                    .iter()
                    .any(|p| p.command == command)
            })
            .map(|e| Message {
                reason: Some(format!("{command} was resolved")),
                ..Message::new(Kind::AheadResolved, &[&e.by], &e.command)
            })
            .collect()
    }

    /// A parked command's author asks for a recompute: the command goes
    /// back at the end of its partition and confirms against the view as
    /// it is now — what the author read stays what they read, so a row
    /// that moved since asks them again. The old entry is withdrawn; the
    /// new one names it.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], or [`Error::NotNow`] for a command not parked.
    pub fn requeue(
        &mut self,
        command: &str,
        confirmed: BTreeMap<String, Value>,
    ) -> Result<(Entry, Vec<Message>)> {
        let old = self.entry_mut(command)?;
        if old.state != State::Parked {
            return Err(Error::NotNow(format!(
                "{command} is {:?}, not parked",
                old.state
            )));
        }
        old.state = State::Cancelled;
        let fresh = Entry {
            command: format!("{command}-r"),
            state: State::Queued,
            confirmation: None,
            parked: None,
            requeued_from: Some(command.to_string()),
            ..old.clone()
        };
        Ok(self.submit(fresh, confirmed, false))
    }

    /// A workflow freezes `partition` from its next position, for
    /// `ttl_seconds`.
    pub fn hold(
        &mut self,
        id: &str,
        partition: &str,
        by: &str,
        ttl_seconds: u64,
        now: u64,
    ) -> Hold {
        let hold = Hold {
            id: id.to_string(),
            partition: partition.to_string(),
            by: by.to_string(),
            position: self.take_position(partition),
            expires: now.saturating_add(ttl_seconds),
            ttl_seconds,
            warned: false,
        };
        self.holds.push(hold.clone());
        hold
    }

    /// The owner renews the hold for another `ttl`.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for an unknown or expired hold.
    pub fn renew(&mut self, id: &str, now: u64) -> Result<&Hold> {
        let hold = self
            .holds
            .iter_mut()
            .find(|h| h.id == id)
            .ok_or_else(|| Error::NotFound(format!("hold {id}")))?;
        hold.expires = now.saturating_add(hold.ttl_seconds);
        hold.warned = false;
        Ok(hold)
    }

    /// The owner releases the hold.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for an unknown hold.
    pub fn release(&mut self, id: &str) -> Result<Hold> {
        let at = self
            .holds
            .iter()
            .position(|h| h.id == id)
            .ok_or_else(|| Error::NotFound(format!("hold {id}")))?;
        Ok(self.holds.remove(at))
    }

    /// The hold that freezes `partition` at or before `position`, if alive.
    fn freezing(&self, partition: &str, position: u64, now: u64) -> Option<&Hold> {
        self.holds
            .iter()
            .filter(|h| h.partition == partition && h.expires > now)
            .find(|h| h.position <= position)
    }

    /// Looks at the head of `partition`: what it is ready for. A head that
    /// reached the front unconfirmed is parked here, and the queue goes on:
    /// a parked entry is set aside, never the head again.
    pub fn advance(&mut self, partition: &str, now: u64) -> (Step, Vec<Message>) {
        let head = self
            .pending_in(partition)
            .into_iter()
            .find(|e| e.state != State::Parked)
            .map(|e| (e.command.clone(), e.position, e.state, e.by.clone()));
        let Some((head, position, state, by)) = head else {
            return (Step::Empty, Vec::new());
        };
        if let Some(hold) = self.freezing(partition, position, now) {
            let message = Message {
                hold: Some(HoldRef {
                    id: hold.id.clone(),
                    by: hold.by.clone(),
                    expires: hold.expires.to_string(),
                }),
                ..Message::new(Kind::BlockedByHold, &[&by, &hold.by], &head)
            };
            return (Step::Blocked(head), vec![message]);
        }
        match state {
            State::Queued | State::Confirmed => (Step::Ready(head), Vec::new()),
            State::AwaitingReview => (Step::Waiting(head), Vec::new()),
            State::AwaitingConfirmation => {
                if let Ok(entry) = self.entry_mut(&head) {
                    entry.state = State::Parked;
                    entry.parked = Some(Parked {
                        ttl_seconds: PARKED_TTL_SECONDS,
                        since: now,
                    });
                }
                let message = Message {
                    reason: Some("reached the head unconfirmed".to_string()),
                    actions: vec![Action::Requeue, Action::Cancel],
                    ..Message::new(Kind::Parked, &[&by], &head)
                };
                (Step::Parked(head), vec![message])
            }
            // The head is pending and not parked, by construction.
            State::Parked
            | State::Applied
            | State::Rejected
            | State::Cancelled
            | State::Expired => (Step::Empty, Vec::new()),
        }
    }

    /// The applier applied the head: `dataVersion` taken, the author and
    /// the infrastructure told, the authors behind told what resolved.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`].
    pub fn applied(&mut self, command: &str, data_version: u64) -> Result<Vec<Message>> {
        let entry = self.entry_mut(command)?;
        entry.state = State::Applied;
        let by = entry.by.clone();
        let partition = entry.partition.clone();
        let mut messages = vec![Message {
            data_version: Some(data_version),
            ..Message::new(Kind::CommandApplied, &[&by, "infrastructure"], command)
        }];
        messages.extend(self.ahead_resolved(&partition, command));
        Ok(messages)
    }

    /// The applier rejected the head, and says why.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`].
    pub fn rejected(
        &mut self,
        command: &str,
        violations: &[String],
        reason: &str,
    ) -> Result<Vec<Message>> {
        let entry = self.entry_mut(command)?;
        entry.state = State::Rejected;
        let by = entry.by.clone();
        let partition = entry.partition.clone();
        let mut messages = vec![Message {
            violations: violations.to_vec(),
            reason: Some(reason.to_string()),
            ..Message::new(Kind::CommandRejected, &[&by], command)
        }];
        messages.extend(self.ahead_resolved(&partition, command));
        Ok(messages)
    }

    /// Time passes: parked commands past their `ttl` expire, holds past
    /// theirs vanish, and the owners near an expiry are warned once.
    pub fn tick(&mut self, now: u64) -> Vec<Message> {
        let mut messages = Vec::new();
        for entry in &mut self.entries {
            let Some(parked) = entry.parked.as_mut() else {
                continue;
            };
            if entry.state != State::Parked {
                continue;
            }
            let expires = parked.since.saturating_add(parked.ttl_seconds);
            if now >= expires {
                entry.state = State::Expired;
                messages.push(Message {
                    reason: Some("parked past its ttl".to_string()),
                    ..Message::new(Kind::Expired, &[&entry.by], &entry.command)
                });
            } else if expires.saturating_sub(now) <= PARKED_WARNING_SECONDS
                && parked.ttl_seconds > PARKED_WARNING_SECONDS
                && now.saturating_sub(parked.since) >= parked.ttl_seconds - PARKED_WARNING_SECONDS
                && now.saturating_sub(parked.since)
                    < parked.ttl_seconds - PARKED_WARNING_SECONDS + 1
            {
                messages.push(Message {
                    actions: vec![Action::Requeue, Action::Cancel],
                    ..Message::new(Kind::ParkedExpiring, &[&entry.by], &entry.command)
                });
            }
        }
        let mut kept = Vec::new();
        for mut hold in self.holds.drain(..) {
            if now >= hold.expires {
                continue;
            }
            if !hold.warned && hold.expires.saturating_sub(now) <= HOLD_WARNING_SECONDS {
                hold.warned = true;
                messages.push(Message {
                    hold: Some(HoldRef {
                        id: hold.id.clone(),
                        by: hold.by.clone(),
                        expires: hold.expires.to_string(),
                    }),
                    actions: vec![Action::Renew],
                    ..Message::new(Kind::HoldExpiring, &[&hold.by], &hold.id)
                });
            }
            kept.push(hold);
        }
        self.holds = kept;
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Effect;

    fn entry(command: &str, by: &str, field: &str, value: i64, mode: Mode) -> Entry {
        Entry {
            command: command.to_string(),
            data_capability: "credit.requestLimitChange@2".to_string(),
            by: by.to_string(),
            partition: "Account/881".to_string(),
            position: 0,
            based_on: BasedOn {
                version: 10,
                values: BTreeMap::from([(field.to_string(), Value::from(5000))]),
            },
            payload: BTreeMap::from([(field.to_string(), Value::from(value))]),
            effect: Effect::Update,
            touches: vec![format!("Account.{field}")],
            mode,
            projection: Projection::default(),
            state: State::Queued,
            confirmation: None,
            parked: None,
            requeued_from: None,
            idempotency_key: None,
        }
    }

    fn row(limit: i64) -> BTreeMap<String, Value> {
        BTreeMap::from([("creditLimit".to_string(), Value::from(limit))])
    }

    fn kinds(messages: &[Message]) -> Vec<Kind> {
        messages.iter().map(|m| m.message).collect()
    }

    #[test]
    fn the_first_command_on_a_fresh_base_is_queued_and_told_so() {
        let mut queue = Queue::default();
        let (alice, said) = queue.submit(
            entry("c_120", "alice", "creditLimit", 8000, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        assert_eq!(alice.position, 0);
        assert_eq!(alice.state, State::Queued);
        assert_eq!(kinds(&said), [Kind::CommandQueued]);
    }

    #[test]
    fn the_contract_s_scenario_a_value_declared_ahead_asks_the_author() {
        // Alice declares 8000; Bob, on the same base, declares 3000: he is told.
        let mut queue = Queue::default();
        queue.submit(
            entry("c_120", "alice", "creditLimit", 8000, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        let (bob, said) = queue.submit(
            entry("c_123", "bob", "creditLimit", 3000, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        assert_eq!(bob.position, 1);
        assert_eq!(bob.state, State::AwaitingConfirmation);
        assert_eq!(
            kinds(&said),
            [Kind::CommandQueued, Kind::ValueDeclaredAhead]
        );
        let told = &said[1];
        assert_eq!(told.to, ["bob"]);
        assert_eq!(told.field.as_deref(), Some("Account/881.creditLimit"));
        assert_eq!(told.your_value, Some(Value::from(3000)));
        let ahead = told.declared_ahead.as_ref().expect("what is ahead");
        assert_eq!(ahead.by, "alice");
        assert_eq!(ahead.value, Value::from(8000));
        assert_eq!(bob.projection.pending_ahead.len(), 1);
        assert_eq!(bob.projection.confirmed, row(5000));
    }

    #[test]
    fn a_value_that_moved_since_the_author_read_it_asks_too() {
        let mut queue = Queue::default();
        // Nobody ahead, but the row is at 8000 and carol read 5000.
        let (carol, said) = queue.submit(
            entry("c_130", "carol", "creditLimit", 4500, Mode::ConfirmOnStale),
            row(8000),
            false,
        );
        assert_eq!(carol.state, State::AwaitingConfirmation);
        let ahead = said[1].declared_ahead.as_ref().expect("the applied value");
        assert_eq!(ahead.by, "(applied)");
        assert_eq!(ahead.value, Value::from(8000));
    }

    #[test]
    fn relative_and_overwrite_commands_are_never_asked() {
        let mut queue = Queue::default();
        queue.submit(
            entry("c_1", "alice", "stock", 10, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        let (relative, said) = queue.submit(
            entry("c_2", "bob", "stock", -2, Mode::Relative),
            row(5000),
            false,
        );
        assert_eq!(relative.state, State::Queued);
        assert_eq!(kinds(&said), [Kind::CommandQueued]);
        let (over, _) = queue.submit(
            entry("c_3", "carol", "stock", 7, Mode::Overwrite),
            row(5000),
            false,
        );
        assert_eq!(over.state, State::Queued);
    }

    #[test]
    fn a_critical_field_waits_for_its_review() {
        let mut queue = Queue::default();
        let (waiting, _) = queue.submit(
            entry("c_1", "alice", "creditLimit", 8000, Mode::ConfirmOnStale),
            row(5000),
            true,
        );
        assert_eq!(waiting.state, State::AwaitingReview);
        let (step, _) = queue.advance("Account/881", 0);
        assert_eq!(step, Step::Waiting("c_1".to_string()));
        queue.reviewed("c_1", true).expect("approved");
        let (step, _) = queue.advance("Account/881", 0);
        assert_eq!(step, Step::Ready("c_1".to_string()));
        let mut other = Queue::default();
        other.submit(
            entry("c_2", "alice", "creditLimit", 8000, Mode::ConfirmOnStale),
            row(5000),
            true,
        );
        assert_eq!(
            other.reviewed("c_2", false).expect("rejected").state,
            State::Rejected
        );
        assert!(matches!(other.reviewed("c_2", true), Err(Error::NotNow(_))));
    }

    #[test]
    fn a_confirmed_command_keeps_its_place_and_an_unconfirmed_head_is_parked() {
        let mut queue = Queue::default();
        queue.submit(
            entry("c_120", "alice", "creditLimit", 8000, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        queue.submit(
            entry("c_123", "bob", "creditLimit", 3000, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        queue.submit(
            entry("c_130", "carol", "creditLimit", 4500, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        // Carol confirms: she keeps position 2.
        let carol = queue.confirm("c_130", "carol").expect("confirmed");
        assert_eq!(carol.state, State::Confirmed);
        assert_eq!(carol.position, 2);
        assert!(matches!(
            queue.confirm("c_130", "carol"),
            Err(Error::NotNow(_))
        ));
        assert!(matches!(
            queue.confirm("c_999", "x"),
            Err(Error::NotFound(_))
        ));
        // Alice is the head, ready.
        let (step, _) = queue.advance("Account/881", 100);
        assert_eq!(step, Step::Ready("c_120".to_string()));
        let said = queue.applied("c_120", 11).expect("applied");
        assert_eq!(
            kinds(&said),
            [
                Kind::CommandApplied,
                Kind::AheadResolved,
                Kind::AheadResolved
            ]
        );
        assert_eq!(said[0].data_version, Some(11));
        assert_eq!(said[0].to, ["alice", "infrastructure"]);
        // Bob reaches the head unconfirmed: parked, and the queue goes on to Carol.
        let (step, said) = queue.advance("Account/881", 100);
        assert_eq!(step, Step::Parked("c_123".to_string()));
        assert_eq!(kinds(&said), [Kind::Parked]);
        assert_eq!(queue.entry("c_123").map(|e| e.state), Some(State::Parked));
        let (step, _) = queue.advance("Account/881", 100);
        assert_eq!(step, Step::Ready("c_130".to_string()));
    }

    #[test]
    fn a_parked_command_comes_back_at_the_end_on_a_new_base_and_expires_otherwise() {
        let mut queue = Queue::default();
        queue.submit(
            entry("c_123", "bob", "creditLimit", 3000, Mode::ConfirmOnStale),
            row(8000),
            false,
        );
        let (_, _) = queue.advance("Account/881", 100);
        assert_eq!(queue.entry("c_123").map(|e| e.state), Some(State::Parked));
        queue.submit(
            entry("c_140", "dave", "creditLimit", 6000, Mode::Overwrite),
            row(8000),
            false,
        );
        let (fresh, said) = queue.requeue("c_123", row(8000)).expect("requeued");
        assert_eq!(fresh.command, "c_123-r");
        assert_eq!(fresh.requeued_from.as_deref(), Some("c_123"));
        assert_eq!(fresh.position, 2, "at the end, behind dave");
        assert_eq!(
            fresh.state,
            State::AwaitingConfirmation,
            "dave is ahead on the same field"
        );
        assert_eq!(
            kinds(&said),
            [Kind::CommandQueued, Kind::ValueDeclaredAhead]
        );
        assert_eq!(
            queue.entry("c_123").map(|e| e.state),
            Some(State::Cancelled)
        );
        assert!(matches!(
            queue.requeue("c_140", row(8000)),
            Err(Error::NotNow(_))
        ));
        // Another parked one, left alone, expires after its ttl.
        let mut other = Queue::default();
        other.submit(
            entry("c_1", "bob", "creditLimit", 3000, Mode::ConfirmOnStale),
            row(8000),
            false,
        );
        other.advance("Account/881", 1000);
        let warned = other.tick(1000 + PARKED_TTL_SECONDS - PARKED_WARNING_SECONDS);
        assert_eq!(kinds(&warned), [Kind::ParkedExpiring]);
        let said = other.tick(1000 + PARKED_TTL_SECONDS);
        assert_eq!(kinds(&said), [Kind::Expired]);
        assert_eq!(other.entry("c_1").map(|e| e.state), Some(State::Expired));
        assert!(
            other.tick(1000 + PARKED_TTL_SECONDS + 1).is_empty(),
            "expected nothing twice"
        );
    }

    #[test]
    fn a_cancelled_command_tells_who_saw_it_ahead() {
        let mut queue = Queue::default();
        queue.submit(
            entry("c_120", "alice", "creditLimit", 8000, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        queue.submit(
            entry("c_123", "bob", "creditLimit", 3000, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        let said = queue.cancel("c_120").expect("cancelled");
        assert_eq!(kinds(&said), [Kind::AheadResolved]);
        assert_eq!(said[0].to, ["bob"]);
        assert!(matches!(queue.cancel("c_120"), Err(Error::NotNow(_))));
        let (step, _) = queue.advance("Account/881", 0);
        assert_eq!(
            step,
            Step::Parked("c_123".to_string()),
            "bob never confirmed"
        );
    }

    #[test]
    fn a_hold_freezes_the_partition_from_its_position_until_it_expires_or_is_released() {
        let mut queue = Queue::default();
        queue.submit(
            entry("c_1", "alice", "creditLimit", 8000, Mode::Overwrite),
            row(5000),
            false,
        );
        let hold = queue.hold("w_77", "Account/881", "credit.limitReview", 60, 1000);
        assert_eq!(hold.position, 1);
        // c_1 is before the hold: not frozen.
        let (step, _) = queue.advance("Account/881", 1000);
        assert_eq!(step, Step::Ready("c_1".to_string()));
        queue.applied("c_1", 11).expect("applied");
        queue.submit(
            entry("c_2", "bob", "creditLimit", 7000, Mode::Overwrite),
            row(8000),
            false,
        );
        let (step, said) = queue.advance("Account/881", 1001);
        assert_eq!(step, Step::Blocked("c_2".to_string()));
        assert_eq!(kinds(&said), [Kind::BlockedByHold]);
        assert_eq!(said[0].to, ["bob", "credit.limitReview"]);
        // Renewed, it holds longer; expired, it lets go.
        queue.renew("w_77", 1050).expect("renewed");
        assert_eq!(queue.holds()[0].expires, 1110);
        let warned = queue.tick(1100);
        assert_eq!(kinds(&warned), [Kind::HoldExpiring]);
        assert!(queue.tick(1101).is_empty(), "warned once");
        let (step, _) = queue.advance("Account/881", 1100);
        assert_eq!(step, Step::Blocked("c_2".to_string()));
        queue.tick(1110);
        assert!(queue.holds().is_empty(), "expected nothing");
        let (step, _) = queue.advance("Account/881", 1110);
        assert_eq!(step, Step::Ready("c_2".to_string()));
        assert!(matches!(queue.renew("w_77", 0), Err(Error::NotFound(_))));
        let again = queue.hold("w_78", "Account/881", "x", 60, 2000);
        assert_eq!(queue.release(&again.id).expect("released").id, "w_78");
        assert!(matches!(queue.release("w_78"), Err(Error::NotFound(_))));
    }

    #[test]
    fn a_rejection_at_the_head_says_why_and_resolves_what_was_behind() {
        let mut queue = Queue::default();
        queue.submit(
            entry("c_1", "alice", "creditLimit", 99_000, Mode::Overwrite),
            row(5000),
            false,
        );
        queue.submit(
            entry("c_2", "bob", "creditLimit", 7000, Mode::ConfirmOnStale),
            row(5000),
            false,
        );
        let said = queue
            .rejected(
                "c_1",
                &["limit-under-ceiling".to_string()],
                "over the ceiling",
            )
            .expect("rejected");
        assert_eq!(kinds(&said), [Kind::CommandRejected, Kind::AheadResolved]);
        assert_eq!(said[0].violations, ["limit-under-ceiling"]);
        assert_eq!(queue.entry("c_1").map(|e| e.state), Some(State::Rejected));
    }

    #[test]
    fn a_loaded_queue_continues_the_positions_it_was_saved_with() {
        let mut queue = Queue::default();
        queue.submit(
            entry("c_1", "alice", "creditLimit", 1, Mode::Overwrite),
            row(0),
            false,
        );
        queue.hold("w_1", "Account/881", "x", 60, 0);
        let loaded = Queue::load(queue.entries().to_vec(), queue.holds().to_vec());
        let mut loaded = loaded;
        let (next, _) = loaded.submit(
            entry("c_2", "bob", "creditLimit", 2, Mode::Overwrite),
            row(0),
            false,
        );
        assert_eq!(next.position, 2);
        assert_eq!(queue.entries().len(), 1);
    }

    #[test]
    fn an_empty_partition_has_no_head() {
        let mut queue = Queue::default();
        assert_eq!(queue.advance("Nobody/1", 0).0, Step::Empty);
    }

    proptest::proptest! {
        #[test]
        fn positions_are_fixed_and_strictly_increasing_whatever_happens(
            authors in proptest::collection::vec(0u8..4, 1..12),
            confirms in proptest::collection::vec(proptest::bool::ANY, 12),
        ) {
            let mut queue = Queue::default();
            let mut ids = Vec::new();
            for (n, author) in authors.iter().enumerate() {
                let id = format!("c_{n}");
                let by = format!("author{author}");
                let n = u64::try_from(n).expect("small");
                let (e, _) = queue.submit(entry(&id, &by, "creditLimit", i64::try_from(n).expect("small"), Mode::ConfirmOnStale), row(5000), false);
                proptest::prop_assert_eq!(e.position, n);
                ids.push(id);
            }
            for (id, confirm) in ids.iter().zip(confirms.iter()) {
                if *confirm {
                    let _ = queue.confirm(id, "someone");
                }
            }
            // Drive the partition to the end: every step either applies the
            // head, parks it, or finds the partition empty.
            let mut applied: Vec<u64> = Vec::new();
            for _ in 0..40 {
                match queue.advance("Account/881", 0).0 {
                    Step::Ready(head) => {
                        let position = queue.entry(&head).expect("entry").position;
                        applied.push(position);
                        queue.applied(&head, 1).expect("applied");
                    }
                    Step::Parked(_) | Step::Waiting(_) | Step::Blocked(_) => {}
                    Step::Empty => break,
                }
            }
            proptest::prop_assert!(applied.windows(2).all(|w| w[0] < w[1]), "{applied:?}");
            // Nothing pending but parked ones remains.
            for e in queue.entries() {
                proptest::prop_assert!(e.state.is_final() || e.state == State::Parked, "{:?}", e.state);
            }
        }
    }
}
