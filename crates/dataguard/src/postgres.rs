//! The store on PostgreSQL: the queue as a table, the journal, the global
//! version, `NOTIFY` on every application so a server streams the
//! `dataVersion` (contract L). Behind the `postgres` feature.
//!
//! Every method runs its own statement, and [`PgStore::apply`] its own
//! transaction: the engine's single applier is the only writer of the
//! version and the rows, so no `FOR UPDATE SKIP LOCKED` is needed on the
//! queue — one server drives one queue. A second server would take the
//! journal as its source of truth and `LISTEN dataguard`.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use crate::command::{CommandResult, Entry};
use crate::queue::Hold;
use crate::store::{Collision, Store, Write};
use crate::{Error, Result};

/// The migration this crate owns: idempotent, run on every start.
pub const MIGRATION: &str = include_str!("../migrations/0001_dataguard.sql");

/// The channel `NOTIFY` speaks on after an application: the payload is the
/// new `dataVersion`.
pub const CHANNEL: &str = "dataguard";

/// The store.
pub struct PgStore {
    client: Mutex<Client>,
}

impl PgStore {
    /// A store over a connected client, its tables created.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the migration fails.
    pub async fn open(client: Client) -> Result<Self> {
        client.batch_execute(MIGRATION).await?;
        Ok(Self {
            client: Mutex::new(client),
        })
    }

    /// The client, for the product's server to `LISTEN` on.
    #[must_use]
    pub const fn client(&self) -> &Mutex<Client> {
        &self.client
    }
}

impl From<tokio_postgres::Error> for Error {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Store(error.to_string())
    }
}

fn json<T: serde::Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| Error::Store(e.to_string()))
}

fn from_json<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|e| Error::Store(e.to_string()))
}

fn signed(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn unsigned(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

const FINAL: [&str; 4] = ["applied", "rejected", "cancelled", "expired"];

#[async_trait]
impl Store for PgStore {
    async fn pending(&self) -> Result<Vec<Entry>> {
        let rows = self.client.lock().await.query(
                "SELECT entry FROM dataguard_queue WHERE NOT (state = ANY($1)) ORDER BY partition, position",
                &[&FINAL.to_vec()],
            )
            .await
            ?;
        rows.into_iter()
            .map(|row| from_json(row.get::<_, Value>(0)))
            .collect()
    }

    async fn holds(&self) -> Result<Vec<Hold>> {
        let rows = self
            .client
            .lock()
            .await
            .query("SELECT hold FROM dataguard_holds ORDER BY id", &[])
            .await?;
        rows.into_iter()
            .map(|row| from_json(row.get::<_, Value>(0)))
            .collect()
    }

    async fn save(&self, entry: &Entry) -> Result<()> {
        let state = json(&entry.state)?;
        let state = state.as_str().unwrap_or("queued").to_string();
        let bare = entry
            .data_capability
            .split('@')
            .next()
            .unwrap_or(&entry.data_capability)
            .to_string();
        self.client.lock().await.execute(
                "INSERT INTO dataguard_queue (command, partition, position, state, data_capability, idempotency_key, entry)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (command) DO UPDATE SET state = EXCLUDED.state, entry = EXCLUDED.entry",
                &[
                    &entry.command as &(dyn ToSql + Sync),
                    &entry.partition,
                    &signed(entry.position),
                    &state,
                    &bare,
                    &entry.idempotency_key,
                    &json(entry)?,
                ],
            )
            .await
            ?;
        Ok(())
    }

    async fn save_hold(&self, hold: &Hold, alive: bool) -> Result<()> {
        if alive {
            self.client
                .lock()
                .await
                .execute(
                    "INSERT INTO dataguard_holds (id, partition, hold) VALUES ($1, $2, $3)
                     ON CONFLICT (id) DO UPDATE SET hold = EXCLUDED.hold",
                    &[
                        &hold.id as &(dyn ToSql + Sync),
                        &hold.partition,
                        &json(hold)?,
                    ],
                )
                .await?;
        } else {
            self.client
                .lock()
                .await
                .execute("DELETE FROM dataguard_holds WHERE id = $1", &[&hold.id])
                .await?;
        }
        Ok(())
    }

    async fn data_version(&self) -> Result<u64> {
        let row = self
            .client
            .lock()
            .await
            .query_one("SELECT version FROM dataguard_version", &[])
            .await?;
        Ok(unsigned(row.get::<_, i64>(0)))
    }

    async fn row(&self, partition: &str) -> Result<BTreeMap<String, Value>> {
        let found = self
            .client
            .lock()
            .await
            .query_opt(
                "SELECT fields FROM dataguard_rows WHERE partition = $1 AND NOT tombstoned",
                &[&partition],
            )
            .await?;
        found.map_or_else(
            || Ok(BTreeMap::new()),
            |row| from_json(row.get::<_, Value>(0)),
        )
    }

    async fn collision(&self, entry: &Entry) -> Result<Collision> {
        // An insert collides when its partition already has a live row: the
        // unique key is the aggregate's id. The product's server refines
        // this with its own constraints.
        let found = self
            .client
            .lock()
            .await
            .query_opt(
                "SELECT tombstoned FROM dataguard_rows WHERE partition = $1",
                &[&entry.partition],
            )
            .await?;
        Ok(found.map_or_else(Collision::default, |row| {
            if row.get::<_, bool>(0) {
                Collision {
                    dangling: vec![entry.partition.clone()],
                    ..Collision::default()
                }
            } else {
                Collision {
                    unique_key: Some(entry.partition.clone()),
                    ..Collision::default()
                }
            }
        }))
    }

    async fn incoming(&self, partition: &str) -> Result<Vec<(String, u64)>> {
        let rows = self.client.lock().await.query(
                "SELECT relation, rows FROM dataguard_links WHERE partition = $1 AND rows > 0 ORDER BY relation",
                &[&partition],
            )
            .await
            ?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get::<_, String>(0), unsigned(row.get::<_, i64>(1))))
            .collect())
    }

    async fn apply(&self, write: &Write, result: &CommandResult) -> Result<u64> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        let row = tx
            .query_one(
                "UPDATE dataguard_version SET version = version + 1 RETURNING version",
                &[],
            )
            .await?;
        let version = unsigned(row.get::<_, i64>(0));
        if write.tombstone {
            tx.execute(
                "INSERT INTO dataguard_rows (partition, tombstoned) VALUES ($1, true)
                 ON CONFLICT (partition) DO UPDATE SET tombstoned = true",
                &[&write.partition],
            )
            .await?;
        } else {
            tx.execute(
                "INSERT INTO dataguard_rows (partition, fields) VALUES ($1, $2)
                 ON CONFLICT (partition) DO UPDATE SET fields = dataguard_rows.fields || EXCLUDED.fields, tombstoned = false",
                &[&write.partition as &(dyn ToSql + Sync), &json(&write.values)?],
            )
            .await
            ?;
        }
        let journaled = CommandResult {
            data_version: Some(version),
            ..result.clone()
        };
        tx.execute(
            "INSERT INTO dataguard_journal (command, data_version, result) VALUES ($1, $2, $3)",
            &[
                &result.command_id as &(dyn ToSql + Sync),
                &signed(version),
                &json(&journaled)?,
            ],
        )
        .await?;
        tx.execute(
            "SELECT pg_notify($1, $2)",
            &[&CHANNEL as &(dyn ToSql + Sync), &version.to_string()],
        )
        .await?;
        tx.commit().await?;
        drop(client);
        Ok(version)
    }

    async fn journal(&self, result: &CommandResult) -> Result<()> {
        self.client
            .lock()
            .await
            .execute(
                "INSERT INTO dataguard_journal (command, data_version, result) VALUES ($1, $2, $3)",
                &[
                    &result.command_id as &(dyn ToSql + Sync),
                    &result.data_version.map(signed),
                    &json(result)?,
                ],
            )
            .await?;
        Ok(())
    }

    async fn result(&self, command: &str) -> Result<Option<CommandResult>> {
        let found = self
            .client
            .lock()
            .await
            .query_opt(
                "SELECT result FROM dataguard_journal WHERE command = $1 ORDER BY seq DESC LIMIT 1",
                &[&command],
            )
            .await?;
        found.map_or_else(
            || Ok(None),
            |row| from_json(row.get::<_, Value>(0)).map(Some),
        )
    }

    async fn by_idempotency_key(&self, data_capability: &str, key: &str) -> Result<Option<String>> {
        let found = self.client.lock().await.query_opt(
                "SELECT command FROM dataguard_queue WHERE data_capability = $1 AND idempotency_key = $2",
                &[&data_capability as &(dyn ToSql + Sync), &key],
            )
            .await
            ?;
        Ok(found.map(|row| row.get::<_, String>(0)))
    }
}

/// A clock on the system time, in seconds since the epoch.
pub struct SystemClock;

impl crate::store::Clock for SystemClock {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migration_creates_every_table_the_store_reads() {
        for table in [
            "dataguard_queue",
            "dataguard_holds",
            "dataguard_rows",
            "dataguard_links",
            "dataguard_journal",
            "dataguard_version",
        ] {
            assert!(
                MIGRATION.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
                "{table}"
            );
        }
        assert_eq!(signed(u64::MAX), i64::MAX);
        assert_eq!(unsigned(-1), 0);
        assert!(crate::store::Clock::now(&SystemClock) > 1_700_000_000);
    }

    /// Against a real PostgreSQL: `DATAGUARD_PG_URL=postgres://… cargo test
    /// --features postgres -- --ignored`. The harness's own Docker service
    /// (`scripts/postgres.sh`) serves one.
    #[tokio::test]
    #[ignore = "needs a PostgreSQL at DATAGUARD_PG_URL"]
    async fn the_contract_s_scenario_runs_on_postgres() {
        let Ok(url) = std::env::var("DATAGUARD_PG_URL") else {
            return;
        };
        let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connected");
        let driver = tokio::spawn(connection);
        client
            .batch_execute("DROP TABLE IF EXISTS dataguard_queue, dataguard_holds, dataguard_rows, dataguard_links, dataguard_journal, dataguard_version")
            .await
            .expect("clean");
        let store = PgStore::open(client).await.expect("opened");
        let result = CommandResult {
            command_id: "c_1".to_string(),
            status: crate::command::State::Applied,
            data_version: None,
            violations: Vec::new(),
            review_id: None,
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
        assert_eq!(
            store
                .collision(&crate::command::Entry {
                    command: "c_2".to_string(),
                    data_capability: "x.y@1".to_string(),
                    by: String::new(),
                    partition: "Account/881".to_string(),
                    position: 0,
                    based_on: crate::command::BasedOn::default(),
                    payload: BTreeMap::new(),
                    effect: crate::command::Effect::Insert,
                    touches: Vec::new(),
                    mode: crate::command::Mode::Overwrite,
                    projection: crate::command::Projection::default(),
                    state: crate::command::State::Queued,
                    confirmation: None,
                    parked: None,
                    requeued_from: None,
                    idempotency_key: None,
                })
                .await
                .expect("read")
                .unique_key
                .as_deref(),
            Some("Account/881")
        );
        driver.abort();
    }
}
