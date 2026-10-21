//! Where the queue, the journal and the data live — the ports, not the
//! tables. The engine decides; a store keeps.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::command::{CommandResult, Entry};
use crate::queue::Hold;

/// What an application writes: the aggregate row's fields after the
/// command, by partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    /// `<Aggregate>/<id>`.
    pub partition: String,
    /// The fields and their new values.
    pub values: BTreeMap<String, Value>,
    /// True for a deletion: the row is archived with a tombstone.
    pub tombstone: bool,
}

impl Write {
    /// The aggregate of its partition: `Account` of `Account/881`.
    #[must_use]
    pub fn aggregate(&self) -> &str {
        self.partition.split('/').next().unwrap_or(&self.partition)
    }
}

/// What the dynamic classification of an effect found at execution: a
/// pure addition that is not one goes to the Resolver instead of failing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collision {
    /// A unique key the insert would collide with.
    pub unique_key: Option<String>,
    /// A trigger that writes into an existing row.
    pub trigger: Option<String>,
    /// A foreign key toward a missing or archived row.
    pub dangling: Vec<String>,
}

impl Collision {
    /// True when the addition is pure after all.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.unique_key.is_none() && self.trigger.is_none() && self.dangling.is_empty()
    }
}

/// The store the engine keeps everything in.
#[async_trait]
pub trait Store: Send + Sync {
    /// Every entry of the queue that is not final, in partition order then
    /// position.
    async fn pending(&self) -> crate::Result<Vec<Entry>>;

    /// Every hold still alive.
    async fn holds(&self) -> crate::Result<Vec<Hold>>;

    /// Keeps `entry` as it is now (a new one, or one whose state moved).
    async fn save(&self, entry: &Entry) -> crate::Result<()>;

    /// Keeps `hold` as it is now, or removes it when `alive` is false.
    async fn save_hold(&self, hold: &Hold, alive: bool) -> crate::Result<()>;

    /// The current `dataVersion`.
    async fn data_version(&self) -> crate::Result<u64>;

    /// The current values of a partition's row: what a command reads as
    /// the confirmed state. Empty for a row that does not exist yet.
    async fn row(&self, partition: &str) -> crate::Result<BTreeMap<String, Value>>;

    /// What an insert would collide with, at execution: the dynamic
    /// classification of a pure addition.
    async fn collision(&self, entry: &Entry) -> crate::Result<Collision>;

    /// The rows that point at `partition`, per relation: `(relation from,
    /// rows)` — what a cascade walks.
    async fn incoming(&self, partition: &str) -> crate::Result<Vec<(String, u64)>>;

    /// Applies `write` and takes the next `dataVersion`, in one transaction
    /// with the journal row of `result`: the single applier's step. A store
    /// whose projections refuse the write rolls it back and answers
    /// [`crate::Error::Refused`].
    async fn apply(&self, write: &Write, result: &CommandResult) -> crate::Result<u64>;

    /// Journals a result that applied nothing (a rejection, an expiry).
    async fn journal(&self, result: &CommandResult) -> crate::Result<()>;

    /// The result a command id got, if it ever got one.
    async fn result(&self, command: &str) -> crate::Result<Option<CommandResult>>;

    /// The command an idempotency key already named, under this
    /// `DataCapability`, if any.
    async fn by_idempotency_key(
        &self,
        data_capability: &str,
        key: &str,
    ) -> crate::Result<Option<String>>;
}

/// A clock the engine reads instants from, seconds since the epoch.
pub trait Clock: Send + Sync {
    /// Now.
    fn now(&self) -> u64;

    /// Now, in milliseconds since the epoch: what a product stamps when two
    /// writes in one second must be told apart. A clock that only knows
    /// seconds answers `now() * 1000`.
    fn now_millis(&self) -> u64 {
        self.now().saturating_mul(1000)
    }
}

/// Where a fresh row id comes from, when a submission names no row.
pub trait Ids: Send + Sync {
    /// An id no row was given yet.
    fn fresh(&self) -> String;
}

/// Fresh row ids as random uuids (v4), behind the `uuid` feature.
#[cfg(feature = "uuid")]
#[derive(Debug, Clone, Copy, Default)]
pub struct RandomIds;

#[cfg(feature = "uuid")]
impl Ids for RandomIds {
    fn fresh(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock that knows seconds only.
    struct Seconds(u64);

    impl Clock for Seconds {
        fn now(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn a_clock_of_seconds_answers_its_milliseconds_by_default() {
        assert_eq!(Seconds(1_700_000_123).now_millis(), 1_700_000_123_000);
        assert_eq!(Seconds(7).now_millis(), 7000);
        assert_eq!(Seconds(u64::MAX).now_millis(), u64::MAX, "saturated");
    }

    #[test]
    fn a_write_names_the_aggregate_of_its_partition() {
        let write = |partition: &str| Write {
            partition: partition.to_string(),
            values: BTreeMap::new(),
            tombstone: false,
        };
        assert_eq!(write("Account/881").aggregate(), "Account");
        assert_eq!(write("Account/a/b").aggregate(), "Account");
        assert_eq!(write("Account").aggregate(), "Account");
    }

    #[cfg(feature = "uuid")]
    #[test]
    fn random_ids_are_distinct_uuids_v4() {
        let ids: Vec<String> = (0..64).map(|_| RandomIds.fresh()).collect();
        for id in &ids {
            let parsed = uuid::Uuid::parse_str(id).expect("a uuid");
            assert_eq!(parsed.get_version_num(), 4, "{id}");
            assert_eq!(parsed.to_string(), *id, "hyphenated, lower case");
        }
        let mut distinct = ids.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), ids.len());
    }

    #[test]
    fn a_collision_is_clean_when_nothing_was_found() {
        assert!(Collision::default().is_clean());
        assert!(
            !Collision {
                unique_key: Some("email".to_string()),
                ..Collision::default()
            }
            .is_clean()
        );
        assert!(
            !Collision {
                dangling: vec!["accountId".to_string()],
                ..Collision::default()
            }
            .is_clean()
        );
    }
}
