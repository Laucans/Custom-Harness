//! The store, the notifier and the clock in memory: what every test runs
//! on, and what the mock-up of a product could run on.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::Value;

use crate::command::{CommandResult, Entry};
use crate::messages::{Message, Notifier};
use crate::queue::Hold;
use crate::store::{Clock, Collision, Ids, Store, Write};
use crate::{Error, Result};

#[derive(Default)]
struct Inner {
    entries: BTreeMap<String, Entry>,
    holds: BTreeMap<String, Hold>,
    rows: BTreeMap<String, BTreeMap<String, Value>>,
    tombstones: Vec<String>,
    journal: Vec<CommandResult>,
    data_version: u64,
    collisions: BTreeMap<String, Collision>,
    incoming: BTreeMap<String, Vec<(String, u64)>>,
    refusals: BTreeMap<String, String>,
}

/// A store in memory.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

impl MemoryStore {
    /// A store whose `partition` row holds these values.
    #[must_use]
    pub fn with_row(self, partition: &str, values: BTreeMap<String, Value>) -> Self {
        self.lock().rows.insert(partition.to_string(), values);
        self
    }

    /// A store that answers `collision` for a command of this
    /// `DataCapability`.
    #[must_use]
    pub fn colliding(self, data_capability: &str, collision: Collision) -> Self {
        self.lock()
            .collisions
            .insert(data_capability.to_string(), collision);
        self
    }

    /// A store where these rows point at `partition`.
    #[must_use]
    pub fn pointed_at(self, partition: &str, incoming: Vec<(String, u64)>) -> Self {
        self.lock().incoming.insert(partition.to_string(), incoming);
        self
    }

    /// A store whose apply transaction refuses every write of `partition`,
    /// as a projection in it would: nothing is written.
    #[must_use]
    pub fn refusing(self, partition: &str, reason: &str) -> Self {
        self.lock()
            .refusals
            .insert(partition.to_string(), reason.to_string());
        self
    }

    /// The journal, oldest first.
    pub fn journal(&self) -> Vec<CommandResult> {
        self.lock().journal.clone()
    }

    /// The row of `partition` as it is now.
    pub fn row_now(&self, partition: &str) -> BTreeMap<String, Value> {
        self.lock().rows.get(partition).cloned().unwrap_or_default()
    }

    /// True when `partition` was archived.
    pub fn is_tombstoned(&self, partition: &str) -> bool {
        self.lock().tombstones.iter().any(|t| t == partition)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A poisoned lock means a test panicked while holding it; the data
        // is still readable, and a store that refuses forever hides the
        // panic that matters.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn pending(&self) -> Result<Vec<Entry>> {
        let mut pending: Vec<Entry> = self
            .lock()
            .entries
            .values()
            .filter(|e| !e.state.is_final())
            .cloned()
            .collect();
        pending.sort_by(|a, b| (&a.partition, a.position).cmp(&(&b.partition, b.position)));
        Ok(pending)
    }

    async fn holds(&self) -> Result<Vec<Hold>> {
        Ok(self.lock().holds.values().cloned().collect())
    }

    async fn save(&self, entry: &Entry) -> Result<()> {
        self.lock()
            .entries
            .insert(entry.command.clone(), entry.clone());
        Ok(())
    }

    async fn save_hold(&self, hold: &Hold, alive: bool) -> Result<()> {
        let mut inner = self.lock();
        if alive {
            inner.holds.insert(hold.id.clone(), hold.clone());
        } else {
            inner.holds.remove(&hold.id);
        }
        drop(inner);
        Ok(())
    }

    async fn data_version(&self) -> Result<u64> {
        Ok(self.lock().data_version)
    }

    async fn row(&self, partition: &str) -> Result<BTreeMap<String, Value>> {
        Ok(self.row_now(partition))
    }

    async fn collision(&self, entry: &Entry) -> Result<Collision> {
        let key = entry
            .data_capability
            .split('@')
            .next()
            .unwrap_or(&entry.data_capability)
            .to_string();
        Ok(self
            .lock()
            .collisions
            .get(&key)
            .cloned()
            .unwrap_or_default())
    }

    async fn incoming(&self, partition: &str) -> Result<Vec<(String, u64)>> {
        Ok(self
            .lock()
            .incoming
            .get(partition)
            .cloned()
            .unwrap_or_default())
    }

    async fn apply(&self, write: &Write, result: &CommandResult) -> Result<u64> {
        let mut inner = self.lock();
        if let Some(reason) = inner.refusals.get(&write.partition) {
            return Err(Error::Refused(crate::plugins::Refusal::unprocessable(
                reason.clone(),
            )));
        }
        inner.data_version = inner.data_version.saturating_add(1);
        let version = inner.data_version;
        if write.tombstone {
            inner.tombstones.push(write.partition.clone());
        } else {
            let row = inner.rows.entry(write.partition.clone()).or_default();
            for (field, value) in &write.values {
                row.insert(field.clone(), value.clone());
            }
        }
        inner.journal.push(CommandResult {
            data_version: Some(version),
            ..result.clone()
        });
        drop(inner);
        Ok(version)
    }

    async fn journal(&self, result: &CommandResult) -> Result<()> {
        self.lock().journal.push(result.clone());
        Ok(())
    }

    async fn result(&self, command: &str) -> Result<Option<CommandResult>> {
        Ok(self
            .lock()
            .journal
            .iter()
            .rev()
            .find(|r| r.command_id == command)
            .cloned())
    }

    async fn by_idempotency_key(&self, data_capability: &str, key: &str) -> Result<Option<String>> {
        Ok(self
            .lock()
            .entries
            .values()
            .find(|e| {
                e.idempotency_key.as_deref() == Some(key)
                    && e.data_capability
                        .starts_with(&format!("{data_capability}@"))
            })
            .map(|e| e.command.clone()))
    }
}

/// A store that fails every call: what the engine does when the store is
/// down.
pub struct BrokenStore;

#[async_trait]
impl Store for BrokenStore {
    async fn pending(&self) -> Result<Vec<Entry>> {
        Err(Error::Store("down".to_string()))
    }
    async fn holds(&self) -> Result<Vec<Hold>> {
        Err(Error::Store("down".to_string()))
    }
    async fn save(&self, _entry: &Entry) -> Result<()> {
        Err(Error::Store("down".to_string()))
    }
    async fn save_hold(&self, _hold: &Hold, _alive: bool) -> Result<()> {
        Err(Error::Store("down".to_string()))
    }
    async fn data_version(&self) -> Result<u64> {
        Err(Error::Store("down".to_string()))
    }
    async fn row(&self, _partition: &str) -> Result<BTreeMap<String, Value>> {
        Err(Error::Store("down".to_string()))
    }
    async fn collision(&self, _entry: &Entry) -> Result<Collision> {
        Err(Error::Store("down".to_string()))
    }
    async fn incoming(&self, _partition: &str) -> Result<Vec<(String, u64)>> {
        Err(Error::Store("down".to_string()))
    }
    async fn apply(&self, _write: &Write, _result: &CommandResult) -> Result<u64> {
        Err(Error::Store("down".to_string()))
    }
    async fn journal(&self, _result: &CommandResult) -> Result<()> {
        Err(Error::Store("down".to_string()))
    }
    async fn result(&self, _command: &str) -> Result<Option<CommandResult>> {
        Err(Error::Store("down".to_string()))
    }
    async fn by_idempotency_key(
        &self,
        _data_capability: &str,
        _key: &str,
    ) -> Result<Option<String>> {
        Err(Error::Store("down".to_string()))
    }
}

/// A notifier that keeps every message, for a test to read back.
#[derive(Default)]
pub struct MemoryNotifier {
    sent: Mutex<Vec<Message>>,
}

impl MemoryNotifier {
    /// Every message sent so far, oldest first.
    pub fn sent(&self) -> Vec<Message> {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The messages addressed to `recipient`.
    pub fn to(&self, recipient: &str) -> Vec<Message> {
        self.sent()
            .into_iter()
            .filter(|m| m.to.iter().any(|t| t == recipient))
            .collect()
    }
}

impl Notifier for MemoryNotifier {
    fn notify(&self, message: &Message) {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(message.clone());
    }
}

/// A clock a test sets, to the second or to the millisecond.
#[derive(Default)]
pub struct FixedClock(AtomicU64);

impl FixedClock {
    /// A clock at this instant, in seconds.
    #[must_use]
    pub const fn at(now: u64) -> Self {
        Self(AtomicU64::new(now.saturating_mul(1000)))
    }

    /// A clock at this instant, in milliseconds.
    #[must_use]
    pub const fn at_millis(now: u64) -> Self {
        Self(AtomicU64::new(now))
    }

    /// Moves the clock, in seconds.
    pub fn set(&self, now: u64) {
        self.set_millis(now.saturating_mul(1000));
    }

    /// Moves the clock, in milliseconds.
    pub fn set_millis(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> u64 {
        self.now_millis() / 1000
    }

    fn now_millis(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Fresh row ids from a counter, `new_1`, `new_2`…: the engine's default,
/// which starts again at every start.
#[derive(Default)]
pub struct Counter(AtomicU64);

impl Ids for Counter {
    fn fresh(&self) -> String {
        format!(
            "new_{}",
            self.0.fetch_add(1, Ordering::SeqCst).saturating_add(1)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::State;

    #[tokio::test]
    async fn an_application_bumps_the_version_writes_the_row_and_journals() {
        let store = MemoryStore::default().with_row(
            "Account/881",
            BTreeMap::from([("creditLimit".to_string(), Value::from(5000))]),
        );
        let result = CommandResult {
            command_id: "c_1".to_string(),
            status: State::Applied,
            data_version: None,
            violations: Vec::new(),
            review_id: None,
            attachment: None,
        };
        let version = store
            .apply(
                &Write {
                    partition: "Account/881".to_string(),
                    values: BTreeMap::from([("creditLimit".to_string(), Value::from(8000))]),
                    tombstone: false,
                },
                &result,
            )
            .await
            .expect("applied");
        assert_eq!(version, 1);
        assert_eq!(store.data_version().await.expect("read"), 1);
        assert_eq!(
            store.row("Account/881").await.expect("read")["creditLimit"],
            Value::from(8000)
        );
        assert_eq!(
            store
                .result("c_1")
                .await
                .expect("read")
                .and_then(|r| r.data_version),
            Some(1)
        );
        assert_eq!(store.result("c_9").await.expect("read"), None);
        let archived = Write {
            partition: "Account/881".to_string(),
            values: BTreeMap::new(),
            tombstone: true,
        };
        store.apply(&archived, &result).await.expect("archived");
        assert!(store.is_tombstoned("Account/881"));
        assert_eq!(store.journal().len(), 2);
    }

    #[tokio::test]
    async fn the_clock_the_notifier_and_the_broken_store_behave() {
        let clock = FixedClock::at(5);
        assert_eq!(clock.now(), 5);
        assert_eq!(clock.now_millis(), 5000);
        clock.set(9);
        assert_eq!(clock.now(), 9);
        clock.set_millis(9_250);
        assert_eq!((clock.now(), clock.now_millis()), (9, 9_250));
        let precise = FixedClock::at_millis(1_999);
        assert_eq!((precise.now(), precise.now_millis()), (1, 1_999));
        let ids = Counter::default();
        assert_eq!([ids.fresh(), ids.fresh()], ["new_1", "new_2"]);
        let notifier = MemoryNotifier::default();
        notifier.notify(&Message::new(
            crate::messages::Kind::CommandQueued,
            &["bob"],
            "c",
        ));
        assert_eq!(notifier.to("bob").len(), 1);
        assert!(notifier.to("alice").is_empty(), "expected nothing");
        assert!(matches!(BrokenStore.pending().await, Err(Error::Store(_))));
        assert!(matches!(
            BrokenStore.data_version().await,
            Err(Error::Store(_))
        ));
    }
}
