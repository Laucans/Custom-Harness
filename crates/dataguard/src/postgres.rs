//! The store on PostgreSQL: the queue as a table, the journal, the global
//! version, `NOTIFY` on every application so a server streams the
//! `dataVersion` (contract L). Behind the `postgres` feature.
//!
//! Every method runs its own statement, and [`PgStore::apply`] its own
//! transaction: the engine's single applier is the only writer of the
//! version and the rows, so no `FOR UPDATE SKIP LOCKED` is needed on the
//! queue — one server drives one queue. A second server would take the
//! journal as its source of truth and `LISTEN dataguard`.
//!
//! The aggregates' projections into the product's own tables
//! ([`ProjectPg`]) run inside that transaction, after the engine's row and
//! before the commit: the write and what follows it commit together, or
//! not at all. A projection that refuses rolls both back, and the store
//! answers [`Error::Refused`], which the engine turns into a rejection.
//!
//! The Capabilities' reads run elsewhere, on a connection of their own
//! ([`ReadOnly`]), in `READ ONLY` transactions: a read can never write.

mod values;

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use crate::command::{CommandResult, Entry};
use crate::plugins::{Plugins, ProjectPg};
use crate::queue::Hold;
use crate::reads::Rows;
use crate::store::{Collision, Store, Write};
use crate::{Error, Result};
use values::Raw;

/// The migration this crate owns: idempotent, run on every start.
pub const MIGRATION: &str = include_str!("../migrations/0001_dataguard.sql");

/// The channel `NOTIFY` speaks on after an application: the payload is the
/// new `dataVersion`.
pub const CHANNEL: &str = "dataguard";

/// The store.
pub struct PgStore {
    client: Mutex<Client>,
    projections: Vec<(String, Arc<dyn ProjectPg>)>,
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
            projections: Vec::new(),
        })
    }

    /// The store, running the units' projections ([`ProjectPg`]) in the
    /// transaction of every write of their aggregate, each warmed first.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when a projection does not warm.
    pub async fn with_projections(mut self, plugins: &Plugins) -> Result<Self> {
        let projections = plugins.0.project_pgs.clone();
        {
            let client = self.client.lock().await;
            for (aggregate, projection) in &projections {
                projection.warm(&client).await.map_err(|e| {
                    Error::Store(format!("the projection of {aggregate} did not warm: {e}"))
                })?;
            }
        }
        self.projections.extend(projections);
        Ok(self)
    }

    /// The client, for the product's server to `LISTEN` on.
    #[must_use]
    pub const fn client(&self) -> &Mutex<Client> {
        &self.client
    }
}

impl From<tokio_postgres::Error> for Error {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Store(described(&error))
    }
}

/// A driver's error with its causes: `db error` alone says nothing, the
/// server's message (`cannot execute INSERT in a read-only transaction`)
/// is its source.
pub(crate) fn described(error: &tokio_postgres::Error) -> String {
    let mut text = error.to_string();
    let mut cause = std::error::Error::source(error);
    while let Some(inner) = cause {
        text.push_str(": ");
        text.push_str(&inner.to_string());
        cause = inner.source();
    }
    text
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
        let aggregate = write.aggregate();
        for (_, projection) in self.projections.iter().filter(|(a, _)| a == aggregate) {
            if let Err(refused) = projection.project(write, &tx).await {
                tx.rollback().await?;
                return Err(refused.into());
            }
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

/// The Capabilities' reads over PostgreSQL: a connection of their own,
/// each query in a `READ ONLY` transaction.
///
/// The variables are bound in the order the Capability lists them, each
/// typed by the parameter it fills; a row comes back as an object, column
/// name to value (the mapping is `values`'s).
pub struct ReadOnly {
    client: Mutex<Client>,
}

impl ReadOnly {
    /// The reads over this connection, apart from the store's.
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client: Mutex::new(client),
        }
    }
}

#[async_trait]
impl Rows for ReadOnly {
    async fn rows(&self, sql: &str, parameters: &[Value]) -> Result<Value> {
        let mut client = self.client.lock().await;
        let tx = client.build_transaction().read_only(true).start().await?;
        let statement = tx.prepare(sql).await?;
        let bound = values::bound(statement.params(), parameters)?;
        let bound: Vec<&(dyn ToSql + Sync)> = bound
            .iter()
            .map(|value| value as &(dyn ToSql + Sync))
            .collect();
        let rows = tx.query(&statement, &bound).await?;
        let mut answer = Vec::with_capacity(rows.len());
        for row in &rows {
            let columns: Vec<(&str, &tokio_postgres::types::Type)> = row
                .columns()
                .iter()
                .map(|column| (column.name(), column.type_()))
                .collect();
            let cells = (0..row.len())
                .map(|index| row.try_get::<_, Raw<'_>>(index).map(|raw| raw.0))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            answer.push(values::object(&columns, &cells)?);
        }
        tx.commit().await?;
        drop(client);
        Ok(Value::Array(answer))
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
    use crate::command::State;
    use crate::plugins::{Answer, Plugin, ProjectionError};

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
        let Some(mut scratch) = Scratch::new("contract").await else {
            return;
        };
        let store = PgStore::open(scratch.client().await).await.expect("opened");
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
                    at_head_since: None,
                    requeued_from: None,
                    idempotency_key: None,
                })
                .await
                .expect("read")
                .unique_key
                .as_deref(),
            Some("Account/881")
        );
        // The migration runs on every start: on the tables it made, it
        // changes nothing.
        store
            .client()
            .lock()
            .await
            .batch_execute(MIGRATION)
            .await
            .expect("idempotent");
        assert_eq!(store.data_version().await.expect("read"), 1, "untouched");
        assert_eq!(
            store.row("Account/881").await.expect("read")["creditLimit"],
            Value::from(8000),
            "untouched"
        );
        scratch.drop().await;
    }

    /// A schema of a test's own on the PostgreSQL at `DATAGUARD_PG_URL`,
    /// dropped at the end: tests running together never see each other's
    /// tables.
    struct Scratch {
        url: String,
        schema: String,
        admin: Client,
        drivers: Vec<tokio::task::JoinHandle<std::result::Result<(), tokio_postgres::Error>>>,
    }

    impl Scratch {
        /// The schema, created; `None` without `DATAGUARD_PG_URL`.
        async fn new(name: &str) -> Option<Self> {
            let url = std::env::var("DATAGUARD_PG_URL").ok()?;
            let schema = format!("dataguard_test_{name}_{}", std::process::id());
            let (admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
                .await
                .expect("connected");
            let driver = tokio::spawn(connection);
            admin
                .batch_execute(&format!(
                    "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema}"
                ))
                .await
                .expect("a scratch schema");
            Some(Self {
                url,
                schema,
                admin,
                drivers: vec![driver],
            })
        }

        /// A connection whose tables are the scratch schema's.
        async fn client(&mut self) -> Client {
            let mut config: tokio_postgres::Config = self.url.parse().expect("a url");
            config.options(format!("-c search_path={}", self.schema));
            let (client, connection) = config
                .connect(tokio_postgres::NoTls)
                .await
                .expect("connected");
            self.drivers.push(tokio::spawn(connection));
            client
        }

        /// One value a query answers, on the scratch schema.
        async fn one<T: for<'a> tokio_postgres::types::FromSql<'a>>(&self, sql: &str) -> T {
            self.admin
                .query_one(&sql.replace("{schema}", &self.schema), &[])
                .await
                .expect("answered")
                .get(0)
        }

        async fn drop(self) {
            self.admin
                .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema))
                .await
                .expect("dropped");
            for driver in self.drivers {
                driver.abort();
            }
        }
    }

    const ACCOUNT: &str = r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"normal"}},"invariants":[],"relations":[]}"#;
    const SET_LIMIT: &str = r#"{"dataCapability":"credit.setLimit","owner":"credit","version":1,"effect":"update","target":{"aggregate":"Account","id":"$accountId"},"touches":["Account.creditLimit"],"mode":"overwrite","callableBy":["*"]}"#;

    /// A product's read table, kept by its aggregate's projection in the
    /// apply transaction; a limit past 9000 is refused.
    struct Ledger;

    #[async_trait]
    impl ProjectPg for Ledger {
        async fn project(
            &self,
            write: &Write,
            tx: &tokio_postgres::Transaction<'_>,
        ) -> std::result::Result<(), ProjectionError> {
            let limit = write
                .values
                .get("creditLimit")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let id = write.partition.split_once('/').map_or("", |(_, id)| id);
            // Written first, refused after: the rollback must undo it.
            tx.execute(
                "INSERT INTO account_limits (id, credit_limit) VALUES ($1, $2)
                 ON CONFLICT (id) DO UPDATE SET credit_limit = EXCLUDED.credit_limit",
                &[&id as &(dyn ToSql + Sync), &limit],
            )
            .await?;
            if limit > 9000 {
                return Err(ProjectionError::Refused(
                    "over the ledger's cap".to_string(),
                ));
            }
            Ok(())
        }

        async fn warm(&self, client: &Client) -> std::result::Result<(), ProjectionError> {
            client
                .batch_execute(
                    "CREATE TABLE IF NOT EXISTS account_limits (id text PRIMARY KEY, credit_limit bigint NOT NULL)",
                )
                .await?;
            Ok(())
        }
    }

    /// An engine over the scratch schema, the ledger projecting `Account`.
    async fn ledger_engine(
        scratch: &mut Scratch,
    ) -> (crate::engine::Engine, Arc<crate::memory::MemoryNotifier>) {
        let plugins: Plugins =
            std::iter::once(Plugin::new().project_pg("Account", Ledger)).collect();
        let store = PgStore::open(scratch.client().await)
            .await
            .expect("opened")
            .with_projections(&plugins)
            .await
            .expect("warmed");
        let notifier = Arc::new(crate::memory::MemoryNotifier::default());
        let engine = crate::engine::Engine::new(
            Arc::new(store),
            notifier.clone(),
            Arc::new(crate::memory::FixedClock::at(1000)),
        )
        .with_aggregate(crate::aggregate::Aggregate::parse(ACCOUNT).expect("I"))
        .with_data_capability(crate::manifest::DataCapability::parse(SET_LIMIT).expect("F"))
        .with_plugins(plugins);
        engine.load().await.expect("fits");
        (engine, notifier)
    }

    fn set_limit(limit: i64) -> crate::api::Submission {
        crate::api::Submission {
            data_capability: "credit.setLimit".to_string(),
            payload: BTreeMap::from([
                ("accountId".to_string(), Value::from("881")),
                ("creditLimit".to_string(), Value::from(limit)),
            ]),
            target_id: None,
            based_on: None,
            idempotency_key: None,
            caller: crate::engine::Engine::caller("alice", "credit.Form", &[], &[]),
        }
    }

    #[tokio::test]
    #[ignore = "needs a PostgreSQL at DATAGUARD_PG_URL"]
    async fn a_projection_writes_the_product_s_table_in_the_apply_transaction() {
        let Some(mut scratch) = Scratch::new("projected").await else {
            return;
        };
        let (engine, _) = ledger_engine(&mut scratch).await;
        engine.submit(set_limit(5000)).await.expect("queued");
        let results = engine.run_once().await.expect("ran");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, State::Applied, "{results:?}");
        assert_eq!(results[0].data_version, Some(1));
        assert_eq!(
            scratch
                .one::<i64>("SELECT credit_limit FROM {schema}.account_limits WHERE id = '881'")
                .await,
            5000,
            "the product's table"
        );
        assert_eq!(
            scratch
                .one::<Value>(
                    "SELECT fields FROM {schema}.dataguard_rows WHERE partition = 'Account/881'"
                )
                .await["creditLimit"],
            Value::from(5000),
            "the engine's row, in the same transaction"
        );
        scratch.drop().await;
    }

    #[tokio::test]
    #[ignore = "needs a PostgreSQL at DATAGUARD_PG_URL"]
    async fn a_refused_projection_rolls_back_the_write_and_the_command_is_rejected() {
        let Some(mut scratch) = Scratch::new("refused").await else {
            return;
        };
        let (engine, notifier) = ledger_engine(&mut scratch).await;
        engine.submit(set_limit(5000)).await.expect("queued");
        engine.run_once().await.expect("applied");
        let over = engine.submit(set_limit(9500)).await.expect("queued");
        let results = engine.run_once().await.expect("the step goes on");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].command_id, over.command_id);
        assert_eq!(results[0].status, State::Rejected, "{results:?}");
        assert_eq!(
            engine.status(&over.command_id).await.expect("known").status,
            State::Rejected,
            "journaled"
        );
        assert_eq!(
            scratch
                .one::<i64>("SELECT credit_limit FROM {schema}.account_limits WHERE id = '881'")
                .await,
            5000,
            "the product's table rolled back"
        );
        assert_eq!(
            scratch
                .one::<Value>(
                    "SELECT fields FROM {schema}.dataguard_rows WHERE partition = 'Account/881'"
                )
                .await["creditLimit"],
            Value::from(5000),
            "the engine's row rolled back"
        );
        assert_eq!(
            scratch
                .one::<i64>("SELECT version FROM {schema}.dataguard_version")
                .await,
            1,
            "no version taken"
        );
        let told: Vec<Option<String>> = notifier
            .to("alice")
            .into_iter()
            .filter(|m| m.message == crate::messages::Kind::CommandRejected)
            .map(|m| m.reason)
            .collect();
        assert_eq!(told, [Some("over the ledger's cap".to_string())]);
        scratch.drop().await;
    }

    #[tokio::test]
    #[ignore = "needs a PostgreSQL at DATAGUARD_PG_URL"]
    async fn read_only_answers_a_capability_s_query_and_never_writes() {
        let Some(mut scratch) = Scratch::new("reads").await else {
            return;
        };
        let setup = scratch.client().await;
        setup
            .batch_execute(
                "CREATE TABLE risk (account text, score int4, small int2, big int8, ratio numeric, f4 float4, f8 float8,
                    ok bool, note varchar(20), data jsonb, id uuid, at timestamptz);
                 INSERT INTO risk VALUES
                    ('881', 2, 7, 9000000000, 12345.678, 0.5, 2.25, true, 'late', '{\"a\": [1]}',
                     '550e8400-e29b-41d4-a716-446655440000', '2026-10-21T10:00:00.25Z'),
                    ('881', 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL),
                    ('882', 3, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);",
            )
            .await
            .expect("a product's table");
        let answer = |capability: &str, sql: &str| Answer {
            capability: capability.to_string(),
            sql: sql.to_string(),
            variables: vec!["accountId".to_string()],
            answer: |rows, _| rows.clone(),
        };
        let plugins: Plugins = [
            Plugin::new().answer(answer(
                "credit.risk",
                "SELECT score, small, big, ratio, f4, f8, ok, note, data, id, at FROM risk WHERE account = $1 ORDER BY score",
            )),
            Plugin::new().answer(answer(
                "credit.sneaky",
                "INSERT INTO risk (account, score) VALUES ($1, 0) RETURNING score",
            )),
            Plugin::new().answer(answer(
                "credit.day",
                "SELECT score, current_date AS day FROM risk WHERE account = $1",
            )),
        ]
        .into_iter()
        .collect();
        let reads = crate::reads::Reads::from(&plugins)
            .with_connection(Arc::new(ReadOnly::new(scratch.client().await)));
        let variables = BTreeMap::from([("accountId".to_string(), Value::from("881"))]);
        let rows = reads
            .run("credit.risk", &variables)
            .await
            .expect("answered");
        assert_eq!(
            rows,
            serde_json::json!([
                { "score": 1, "small": null, "big": null, "ratio": null, "f4": null, "f8": null,
                  "ok": null, "note": null, "data": null, "id": null, "at": null },
                { "score": 2, "small": 7, "big": 9_000_000_000_i64, "ratio": "12345.678", "f4": 0.5,
                  "f8": 2.25, "ok": true, "note": "late", "data": { "a": [1] },
                  "id": "550e8400-e29b-41d4-a716-446655440000", "at": "2026-10-21T10:00:00.250000Z" },
            ])
        );
        let refused = reads.run("credit.sneaky", &variables).await;
        assert!(
            matches!(&refused, Err(Error::Store(why)) if why.contains("read-only transaction")),
            "{refused:?}"
        );
        let unmapped = reads.run("credit.day", &variables).await;
        assert!(
            matches!(&unmapped, Err(Error::Store(why)) if why.starts_with("column day:")),
            "{unmapped:?}"
        );
        assert_eq!(
            scratch
                .one::<i64>("SELECT count(*) FROM {schema}.risk")
                .await,
            3,
            "nothing written"
        );
        assert_eq!(
            reads
                .run(
                    "credit.risk",
                    &BTreeMap::from([("accountId".to_string(), Value::from("882"))])
                )
                .await
                .expect("the connection survives a refusal")
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        scratch.drop().await;
    }

    #[cfg(feature = "uuid")]
    #[tokio::test]
    #[ignore = "needs a PostgreSQL at DATAGUARD_PG_URL"]
    async fn random_ids_are_distinct_uuids_postgres_accepts() {
        use crate::store::{Ids, RandomIds};
        let Some(mut scratch) = Scratch::new("ids").await else {
            return;
        };
        let client = scratch.client().await;
        client
            .batch_execute("CREATE TABLE ids (id uuid PRIMARY KEY)")
            .await
            .expect("a table");
        for _ in 0..200 {
            client
                .execute(
                    "INSERT INTO ids (id) VALUES ($1::text::uuid)",
                    &[&RandomIds.fresh()],
                )
                .await
                .expect("a fresh, valid uuid");
        }
        assert_eq!(
            scratch
                .one::<i64>("SELECT count(DISTINCT id) FROM {schema}.ids WHERE substr(id::text, 15, 1) = '4'")
                .await,
            200,
            "distinct, all version 4"
        );
        scratch.drop().await;
    }
}
