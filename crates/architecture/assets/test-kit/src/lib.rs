//! What a unit's tests would otherwise retype — installed by `harness
//! init-repo`, and the harness's: never edit it here, contest it.
//!
//! The first product's forty-eight `DataCapabilities` each carried their own
//! copy of the same helpers: the aggregate parsed, the plugins collected, the
//! engine planted over memory on a fixed clock, a `submit` writing the wire
//! body, a `settled` running the applier, a refusal unwrapped, a scratch
//! database created, migrated and seeded by hand, the planters' manifests as
//! JSON literals. Every copy drifted a little. They are here once:
//!
//! ```ignore
//! use platform_test_kit::{Bench, planter};
//! use serde_json::json;
//!
//! const AGGREGATE: &str = include_str!("../../../aggregates/Token/aggregate.json");
//!
//! #[tokio::test]
//! async fn a_moved_token_stands_on_its_new_hex() {
//!     let bench = Bench::new(
//!         include_str!("../data-capability.json"),
//!         AGGREGATE,
//!         [aggregate_token::plugin(), move_token::plugin()],
//!     )
//!     .planting(&planter("test.dropToken", "Token", "$tokenId", "insert"))
//!     .memory()
//!     .await;
//!     bench
//!         .plant("test.dropToken", [json!({ "tokenId": TOKEN, "q": 0, "r": 0 })])
//!         .await;
//!     bench
//!         .applied("map.moveToken", json!({ "q": 2, "r": 3 }), Some(TOKEN))
//!         .await;
//!     assert_eq!(bench.row(&format!("Token/{TOKEN}"))["q"], 2);
//! }
//! ```
//!
//! Over PostgreSQL, the same bench on a [`Scratch`]: a database of its own
//! (`platform-scratch-db`), migrated by the product's Data layer
//! (`data::migrate` over `crates/data/migrations/`), seeded with the rows the
//! tables' foreign keys ask for, dropped when the test ends, panicking or
//! not. A Capability's guard checks are [`read`]'s: what the gate
//! `capability-conventions` judges, said before the pull request.
//!
//! A dev-dependency only: `platform-test-kit = { path = "<to the
//! root>/crates/platform/test-kit" }`. Every helper panics with what went
//! wrong: it runs in a test, where a panic is the failure.

#![deny(unsafe_code)]
#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use harness_dataguard::aggregate::Aggregate;
use harness_dataguard::api::{Submission, Submitted};
use harness_dataguard::command::{CommandResult, State};
use harness_dataguard::engine::Engine;
use harness_dataguard::guard::Caller;
use harness_dataguard::manifest::DataCapability;
use harness_dataguard::memory::{FixedClock, MemoryNotifier, MemoryStore};
use harness_dataguard::plugins::{Plugin, Plugins, RefusalKind, Shared};
use harness_dataguard::postgres::{PgStore, ReadOnly};
use harness_dataguard::reads::Rows;
use harness_dataguard::store::{Ids, Store};
use harness_dataguard::tokio_postgres::Client;
use harness_dataguard::{Error, Result};
use platform_scratch_db::ScratchDb;
use serde_json::{Value, json};

/// One partition's row, as the engine holds it.
pub type Row = BTreeMap<String, Value>;

/// Where every bench's clock starts, in seconds since the epoch.
pub const NOW: u64 = 1_792_000_000;

/// A fixture id in the house style, `<prefix>000000-0000-4000-8000-<n>`.
///
/// One prefix per kind of row, `n` the how-many-th — `uuid(0xc1, 1)` is
/// `c1000000-0000-4000-8000-000000000001`: a valid uuid that reads at a
/// glance in an assertion.
#[must_use]
pub fn uuid(prefix: u8, n: u64) -> String {
    format!("{prefix:02x}000000-0000-4000-8000-{n:012x}")
}

/// A file of the product, read from its root: the aggregate's document a
/// unit's tests plant it beside — a unit includes nothing outside its
/// folder (`scripts/isolation.sh`).
///
/// # Panics
///
/// When the file is missing.
#[must_use]
pub fn file(path: &str) -> String {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(path);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("the product's {}: {e}", full.display()))
}

/// A `DataCapability`'s manifest (contract F), parsed.
///
/// # Panics
///
/// When `json` is not one.
#[must_use]
pub fn manifest(json: &str) -> DataCapability {
    DataCapability::parse(json)
        .unwrap_or_else(|e| panic!("not a DataCapability manifest (contract F): {e}"))
}

/// An aggregate's document (contract I), parsed.
///
/// # Panics
///
/// When `json` is not one.
#[must_use]
pub fn aggregate(json: &str) -> Aggregate {
    Aggregate::parse(json).unwrap_or_else(|e| panic!("not an aggregate (contract I): {e}"))
}

/// The manifest of a test-only `DataCapability` that plants rows for a test:
/// `id` (`test.<verb>`) does `effect` on `aggregate`, its row named by
/// `target` (`$<payload field>`), callable by anyone, overwriting.
#[must_use]
pub fn planter(id: &str, aggregate: &str, target: &str, effect: &str) -> String {
    json!({
        "dataCapability": id,
        "owner": "dataguard",
        "version": 1,
        "effect": effect,
        "target": { "aggregate": aggregate, "id": target },
        "mode": "overwrite",
        "callableBy": ["*"],
    })
    .to_string()
}

/// The message of `answer`, a refusal of `kind`.
///
/// # Panics
///
/// When `answer` is anything else: queued, or refused otherwise.
#[must_use]
pub fn refused(answer: Result<Submitted>, kind: RefusalKind) -> String {
    match answer {
        Err(Error::Refused(refusal)) if refusal.kind == kind => refusal.message,
        other => panic!("a {kind:?} refusal, not {other:?}"),
    }
}

/// Asserts that `payload`, what a `Prepare` writes, holds only fields the
/// manifest's `touches` declares (`<Aggregate>.<field>`).
///
/// The engine judges derived and critical fields by `touches`, never by
/// the payload. A manifest touching nothing (an insert) writes a new row:
/// nothing to compare.
///
/// # Panics
///
/// When the manifest does not parse as JSON, or a field is undeclared.
pub fn writes_only_touches(manifest: &str, payload: &Row) {
    let json: Value =
        serde_json::from_str(manifest).unwrap_or_else(|e| panic!("the manifest is not JSON: {e}"));
    let touches: Vec<&str> = json["touches"]
        .as_array()
        .map(|all| all.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if touches.is_empty() {
        return;
    }
    let declared: Vec<&str> = touches
        .iter()
        .map(|touch| touch.rsplit_once('.').map_or(*touch, |(_, field)| field))
        .collect();
    for field in payload.keys() {
        assert!(
            declared.contains(&field.as_str()),
            "`{field}` is written but not declared in `touches` ({touches:?})"
        );
    }
}

/// The engine as the server plants a unit: its manifest beside its
/// aggregate, the test-only planters, the plugins — over memory
/// ([`Self::memory`]) or a [`Scratch`] ([`Self::postgres`]).
pub struct Bench {
    manifests: Vec<DataCapability>,
    aggregates: Vec<Aggregate>,
    plugins: Vec<Plugin>,
    shared: Option<Shared>,
    ids: Option<Arc<dyn Ids>>,
    caller: Caller,
}

impl Bench {
    /// The unit's manifest (`include_str!("../data-capability.json")`)
    /// beside its aggregate's document, with the plugins of both —
    /// `[aggregate_x::plugin(), my_unit::plugin()]`. Submissions come from
    /// the caller `dm`, from `test`, holding `role:approver`.
    ///
    /// # Panics
    ///
    /// When the manifest or the document does not parse.
    #[must_use]
    pub fn new(manifest: &str, aggregate: &str, plugins: impl IntoIterator<Item = Plugin>) -> Self {
        Self {
            manifests: vec![crate::manifest(manifest)],
            aggregates: vec![crate::aggregate(aggregate)],
            plugins: plugins.into_iter().collect(),
            shared: None,
            ids: None,
            caller: Engine::caller("dm", "test", &[], &["role:approver"]),
        }
    }

    /// A test-only `DataCapability` beside the unit ([`planter`]).
    ///
    /// # Panics
    ///
    /// When the manifest does not parse.
    #[must_use]
    pub fn planting(mut self, manifest: &str) -> Self {
        self.manifests.push(crate::manifest(manifest));
        self
    }

    /// Another aggregate, one a planter writes.
    ///
    /// # Panics
    ///
    /// When the document does not parse.
    #[must_use]
    pub fn beside(mut self, aggregate: &str) -> Self {
        self.aggregates.push(crate::aggregate(aggregate));
        self
    }

    /// What the server shares with the units (a port's fake).
    #[must_use]
    pub fn sharing(mut self, shared: Shared) -> Self {
        self.shared = Some(shared);
        self
    }

    /// Where a fresh row id comes from, in place of the engine's counter.
    #[must_use]
    pub fn ids(mut self, ids: Arc<dyn Ids>) -> Self {
        self.ids = Some(ids);
        self
    }

    /// Who submits, in place of `dm` from `test`.
    #[must_use]
    pub fn caller(mut self, caller: Caller) -> Self {
        self.caller = caller;
        self
    }

    /// The engine over `store`, on `clock`, with everything declared.
    fn engine(
        self,
        store: Arc<dyn Store>,
        clock: Arc<FixedClock>,
        plugins: Plugins,
    ) -> (Engine, Caller) {
        let Self {
            manifests,
            aggregates,
            shared,
            ids,
            caller,
            ..
        } = self;
        let mut engine = Engine::new(store, Arc::new(MemoryNotifier::default()), clock);
        for aggregate in aggregates {
            engine = engine.with_aggregate(aggregate);
        }
        for manifest in manifests {
            engine = engine.with_data_capability(manifest);
        }
        if let Some(shared) = shared {
            engine = engine.with_shared(shared);
        }
        if let Some(ids) = ids {
            engine = engine.with_ids(ids);
        }
        (engine.with_plugins(plugins), caller)
    }

    /// The engine over the in-memory store, loaded: `load` judged the
    /// manifest against its aggregate's document, as the server does.
    ///
    /// # Panics
    ///
    /// When the engine refuses what it was given.
    pub async fn memory(mut self) -> Served {
        let plugins: Plugins = std::mem::take(&mut self.plugins).into_iter().collect();
        let store = Arc::new(MemoryStore::default());
        let clock = Arc::new(FixedClock::at(NOW));
        let (engine, caller) = self.engine(store.clone(), clock.clone(), plugins);
        engine
            .load()
            .await
            .unwrap_or_else(|e| panic!("the engine refuses the unit beside its aggregate: {e}"));
        Served {
            engine,
            clock,
            store: Some(store),
            caller,
        }
    }

    /// The engine over `scratch`'s tables, loaded: the aggregates'
    /// projections warmed, the `Prepare`s reading SQL through a read-only
    /// connection of their own.
    ///
    /// # Panics
    ///
    /// When the store does not open or warm, or the engine refuses what it
    /// was given.
    pub async fn postgres(mut self, scratch: &Scratch) -> Served {
        let plugins: Plugins = std::mem::take(&mut self.plugins).into_iter().collect();
        let store = PgStore::open(scratch.connect().await)
            .await
            .unwrap_or_else(|e| panic!("the PostgreSQL store does not open: {e}"))
            .with_projections(&plugins)
            .await
            .unwrap_or_else(|e| panic!("a projection does not warm: {e}"));
        let rows: Arc<dyn Rows> = Arc::new(ReadOnly::new(scratch.connect().await));
        let clock = Arc::new(FixedClock::at(NOW));
        let (engine, caller) = self.engine(Arc::new(store), clock.clone(), plugins);
        let engine = engine.with_rows(rows);
        engine
            .load()
            .await
            .unwrap_or_else(|e| panic!("the engine refuses the unit beside its aggregate: {e}"));
        Served {
            engine,
            clock,
            store: None,
            caller,
        }
    }
}

/// A planted engine, and what a test does with it.
pub struct Served {
    /// The engine, for what the helpers do not cover.
    pub engine: Engine,
    /// Its clock, which a test moves (`clock.set(..)`).
    pub clock: Arc<FixedClock>,
    /// The in-memory store, over memory; `None` over PostgreSQL, whose
    /// tables are read through [`Scratch::rows`].
    pub store: Option<Arc<MemoryStore>>,
    caller: Caller,
}

impl Served {
    /// Submits `payload` to `data_capability` (`<id>` or `<id>@<version>`)
    /// on the row `target`, as the bench's caller: queued, or refused
    /// before anything is.
    ///
    /// # Errors
    ///
    /// What the engine answers: a [`Error::Refused`] from the unit's
    /// `Prepare`, or the guard's.
    ///
    /// # Panics
    ///
    /// When `payload` is neither a JSON object nor `null`.
    pub async fn submit(
        &self,
        data_capability: &str,
        payload: Value,
        target: Option<&str>,
    ) -> Result<Submitted> {
        let payload = match payload {
            Value::Object(fields) => fields.into_iter().collect(),
            Value::Null => BTreeMap::new(),
            other => panic!("a payload is a JSON object, not {other}"),
        };
        self.engine
            .submit(Submission {
                data_capability: data_capability.to_string(),
                payload,
                target_id: target.map(str::to_string),
                caller: self.caller.clone(),
                ..Submission::default()
            })
            .await
    }

    /// Runs the applier once: what every queued command became.
    ///
    /// # Panics
    ///
    /// When the applier fails.
    pub async fn settled(&self) -> Vec<CommandResult> {
        self.engine
            .run_once()
            .await
            .unwrap_or_else(|e| panic!("the applier failed: {e}"))
    }

    /// One submission, queued then settled: what it became.
    ///
    /// # Panics
    ///
    /// When it is refused, or the applier settles anything but it alone.
    pub async fn applied(
        &self,
        data_capability: &str,
        payload: Value,
        target: Option<&str>,
    ) -> CommandResult {
        if let Err(e) = self.submit(data_capability, payload, target).await {
            panic!("{data_capability} was refused: {e}");
        }
        let mut settled = self.settled().await;
        assert_eq!(settled.len(), 1, "one command settled: {settled:?}");
        settled.remove(0)
    }

    /// Plants a row per payload through the planter `data_capability`, each
    /// naming its own row, then settles them.
    ///
    /// # Panics
    ///
    /// When one is refused, or not applied.
    pub async fn plant(&self, data_capability: &str, payloads: impl IntoIterator<Item = Value>) {
        for payload in payloads {
            if let Err(e) = self.submit(data_capability, payload, None).await {
                panic!("the planter {data_capability} was refused: {e}");
            }
        }
        let planted = self.settled().await;
        assert!(
            planted.iter().all(|result| result.status == State::Applied),
            "every planted row applied: {planted:?}"
        );
    }

    /// The row of `partition` (`<Aggregate>/<id>`) as the store holds it:
    /// empty when there is none.
    ///
    /// # Panics
    ///
    /// Over PostgreSQL: its tables are read through [`Scratch::rows`].
    #[must_use]
    pub fn row(&self, partition: &str) -> Row {
        self.store.as_ref().map_or_else(
            || panic!("a PostgreSQL bench reads its tables through Scratch::rows"),
            |store| store.row_now(partition),
        )
    }
}

/// A test's database: created on `DATABASE_URL`'s server, migrated as the
/// server migrates it, dropped when the test ends — panicking or not.
pub struct Scratch {
    db: ScratchDb,
    client: Client,
}

impl Scratch {
    /// A database for the test `label` describes, every migration of
    /// `crates/data/migrations/` applied by the Data layer's `migrate`.
    ///
    /// # Panics
    ///
    /// When `DATABASE_URL` names no PostgreSQL that lets this role create a
    /// database, or a migration fails.
    pub async fn new(label: &str) -> Self {
        let db = ScratchDb::create(label)
            .await
            .unwrap_or_else(|e| panic!("DATABASE_URL names the harness's PostgreSQL: {e}"));
        let mut client = db
            .connect()
            .await
            .unwrap_or_else(|e| panic!("the scratch database answers: {e}"));
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/migrations");
        data::migrate(&mut client, &migrations)
            .await
            .unwrap_or_else(|e| panic!("the migrations apply: {e}"));
        Self { db, client }
    }

    /// Runs `sql`, the rows the tables' foreign keys ask for before a test.
    ///
    /// # Panics
    ///
    /// When PostgreSQL refuses it.
    pub async fn seed(&self, sql: &str) {
        self.client
            .batch_execute(sql)
            .await
            .unwrap_or_else(|e| panic!("the seed is refused: {e}"));
    }

    /// The connection the database was migrated over.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.client
    }

    /// `DATABASE_URL`, naming this database: for what opens its own pool.
    #[must_use]
    pub fn url(&self) -> &str {
        self.db.url()
    }

    /// A new connection to this database.
    ///
    /// # Panics
    ///
    /// When the server refuses it.
    pub async fn connect(&self) -> Client {
        self.db
            .connect()
            .await
            .unwrap_or_else(|e| panic!("the scratch database answers: {e}"))
    }

    /// What `sql` answers with `parameters`, each row a JSON object keyed
    /// by column — the mapping the engine's `Prepare`s read SQL with.
    ///
    /// # Panics
    ///
    /// When PostgreSQL refuses the query.
    pub async fn rows(&self, sql: &str, parameters: &[Value]) -> Vec<Value> {
        let answer = ReadOnly::new(self.connect().await)
            .rows(sql, parameters)
            .await
            .unwrap_or_else(|e| panic!("the query is refused: {e}"));
        match answer {
            Value::Array(rows) => rows,
            other => vec![other],
        }
    }
}

/// A Capability's guard checks: what the gate `capability-conventions`
/// judges of its `SQL`, its `VARIABLES` and its persisted query, said by a
/// test before the pull request.
pub mod read {
    use std::collections::BTreeSet;
    use std::path::Path;

    use serde_json::Value;

    /// The mock-up's answer for `capability`, `mock/fixtures/<id>.json`.
    ///
    /// # Panics
    ///
    /// When the file is missing or is not JSON.
    #[must_use]
    pub fn fixture(capability: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../mock/fixtures")
            .join(format!("{capability}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("the mock-up's fixture {}: {e}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is JSON: {e}", path.display()))
    }

    /// The persisted query `crates/<system>/queries/<name>.graphql`, read
    /// from the root — a Capability's crate includes nothing outside its
    /// folder (`scripts/isolation.sh`).
    ///
    /// # Panics
    ///
    /// When the file is missing.
    #[must_use]
    pub fn query(system: &str, name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(format!("crates/{system}/queries/{name}.graphql"));
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("the persisted query {}: {e}", path.display()))
    }

    /// The words of `text`: runs of letters, digits and `_`.
    fn words(text: &str) -> impl Iterator<Item = &str> {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
    }

    /// Asserts that `sql` is one read statement: it starts with `SELECT` or
    /// `WITH`, holds no `;`, and no word that writes.
    ///
    /// # Panics
    ///
    /// When it is not.
    pub fn one_read_statement(sql: &str) {
        let upper = sql.trim_start().to_ascii_uppercase();
        assert!(
            upper.starts_with("SELECT") || upper.starts_with("WITH"),
            "the SQL starts with SELECT or WITH: {sql}"
        );
        assert!(!sql.contains(';'), "one statement, no `;`: {sql}");
        let writes = [
            "INSERT", "UPDATE", "DELETE", "MERGE", "ALTER", "DROP", "GRANT",
        ];
        for word in words(&upper) {
            assert!(!writes.contains(&word), "the SQL writes ({word}): {sql}");
        }
    }

    /// The parameters `$1`, `$2`… `sql` names, as numbers.
    fn parameters(sql: &str) -> BTreeSet<usize> {
        sql.split('$')
            .skip(1)
            .filter_map(|after| {
                let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().ok()
            })
            .collect()
    }

    /// Asserts that `sql` binds exactly `$1`…`$n` for the `n` request
    /// variables, each compared as text.
    ///
    /// `<column>::text = $n`, or `= lower($n)`: an id that is no uuid then
    /// answers empty instead of failing. The registry binds `variables` by
    /// position.
    ///
    /// # Panics
    ///
    /// When a parameter is missing, extra, or compared otherwise.
    pub fn binds_in_order(sql: &str, variables: &[&str]) {
        let expected: BTreeSet<usize> = (1..=variables.len()).collect();
        assert_eq!(
            parameters(sql),
            expected,
            "the SQL binds $1..${} for {variables:?}: {sql}",
            variables.len()
        );
        for n in 1..=variables.len() {
            assert!(
                compared_as_text(sql, n),
                "${n} ({}) is compared as text, `<column>::text = ${n}`: {sql}",
                variables[n - 1]
            );
        }
    }

    /// Whether `sql` compares `$n` as text somewhere: `::text`, an `=`,
    /// then `$n` or `lower($n)`, spaces allowed between.
    fn compared_as_text(sql: &str, n: usize) -> bool {
        let wanted = n.to_string();
        sql.match_indices("::text").any(|(at, cast)| {
            let rest = sql[at + cast.len()..].trim_start();
            let Some(rest) = rest.strip_prefix('=') else {
                return false;
            };
            let rest = rest.trim_start();
            let rest = rest.strip_prefix("lower(").map_or(rest, str::trim_start);
            rest.strip_prefix('$')
                .and_then(|after| after.strip_prefix(wanted.as_str()))
                .is_some_and(|after| !after.starts_with(|c: char| c.is_ascii_digit()))
        })
    }

    /// `camelCase` as `camel_case`.
    fn snake(name: &str) -> String {
        let mut out = String::new();
        for (i, c) in name.chars().enumerate() {
            if c.is_ascii_uppercase() && i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        }
        out
    }

    /// Asserts that the persisted query `query` (its `.graphql` text) is
    /// the read `sql` makes: its variables are `variables`, in order, and
    /// every field it selects (`camelCase`) is a word of the SQL in
    /// `snake_case`.
    ///
    /// # Panics
    ///
    /// When they disagree.
    pub fn query_file_matches(sql: &str, query: &str, variables: &[&str]) {
        let body: Vec<&str> = query
            .lines()
            .skip_while(|line| !line.trim_start().starts_with("query"))
            .collect();
        let declared: Vec<String> = body
            .first()
            .map(|line| {
                line.split('$')
                    .skip(1)
                    .filter_map(|after| after.split(':').next())
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty() && !name.contains(' '))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            declared, variables,
            "the query file's variables are VARIABLES, in order"
        );
        let in_sql: BTreeSet<&str> = words(sql).collect();
        for line in body.iter().skip(1) {
            let field = line.trim();
            if field.is_empty() || !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let column = snake(field);
            assert!(
                in_sql.contains(column.as_str()),
                "the query file selects `{field}`, and the SQL has no `{column}`: {sql}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_house_id_reads_at_a_glance() {
        assert_eq!(uuid(0xc1, 1), "c1000000-0000-4000-8000-000000000001");
        assert_eq!(uuid(0x0b, 10), "0b000000-0000-4000-8000-00000000000a");
    }

    #[test]
    fn a_planter_is_a_manifest_the_engine_reads() {
        let text = planter("test.dropToken", "Token", "$tokenId", "insert");
        let parsed = manifest(&text);
        assert_eq!(parsed.data_capability, "test.dropToken");
        let json: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(
            json["target"],
            json!({ "aggregate": "Token", "id": "$tokenId" })
        );
        assert_eq!(json["effect"], "insert");
        assert_eq!(json["mode"], "overwrite");
        assert_eq!(json["callableBy"], json!(["*"]));
    }

    const TOKEN: &str = r#"{"aggregate":"Token","fields":{"q":{"kind":"authoritative","criticality":"normal"}},"invariants":[],"relations":[]}"#;
    const MOVE: &str = r#"{"dataCapability":"map.moveToken","owner":"dataguard","version":1,"effect":"update","target":{"aggregate":"Token","id":"$tokenId"},"touches":["Token.q"],"mode":"overwrite","callableBy":["*"]}"#;

    #[test]
    fn what_a_prepare_writes_is_held_to_the_touches() {
        let row = |field: &str| Row::from([(field.to_string(), json!(1))]);
        writes_only_touches(MOVE, &row("q"));
        let undeclared = std::panic::catch_unwind(|| writes_only_touches(MOVE, &row("r")));
        assert!(undeclared.is_err());
        writes_only_touches(&planter("t.p", "Token", "$id", "insert"), &row("anything"));
    }

    #[tokio::test]
    async fn a_bench_plants_moves_and_refuses_over_memory() {
        let bench = Bench::new(MOVE, TOKEN, [])
            .planting(&planter("test.dropToken", "Token", "$tokenId", "insert"))
            .sharing(Shared::default())
            .memory()
            .await;
        let id = uuid(0x70, 1);
        bench
            .plant("test.dropToken", [json!({ "tokenId": id, "q": 0 })])
            .await;
        let moved = bench
            .applied("map.moveToken@1", json!({ "q": 4 }), Some(&id))
            .await;
        assert_eq!(moved.status, State::Applied, "{moved:?}");
        assert_eq!(bench.row(&format!("Token/{id}"))["q"], 4);
        assert!(bench.row("Token/none").is_empty());
        assert_eq!(bench.settled().await.len(), 0, "nothing queued");
        let unknown = bench.submit("map.nothing", json!({}), None).await;
        assert!(unknown.is_err());
        let wrong = std::panic::catch_unwind(|| {
            refused(Err(Error::Store("x".into())), RefusalKind::NotFound)
        });
        assert!(wrong.is_err());
    }

    #[test]
    fn the_guard_checks_pass_a_canonical_read_and_name_a_drift() {
        let sql = "SELECT s.id::text AS id, s.planned_on FROM session s WHERE s.id::text = $1";
        read::one_read_statement(sql);
        read::binds_in_order(sql, &["id"]);
        let query = "# fields: session.id, session.plannedOn\nquery GetSession($id: ID!) {\n  session(id: $id) {\n    id\n    plannedOn\n  }\n}\n";
        read::query_file_matches(sql, query, &["id"]);
        for drift in [
            "UPDATE session SET id = $1",
            "SELECT 1; SELECT 2",
            "  delete from session",
        ] {
            assert!(
                std::panic::catch_unwind(|| read::one_read_statement(drift)).is_err(),
                "{drift}"
            );
        }
        assert!(
            std::panic::catch_unwind(|| read::binds_in_order(sql, &["id", "campaign"])).is_err()
        );
        assert!(
            std::panic::catch_unwind(|| read::binds_in_order("SELECT 1 WHERE id = $1", &["id"]))
                .is_err()
        );
        assert!(
            std::panic::catch_unwind(|| read::binds_in_order(
                "SELECT 1 WHERE id::text = $12",
                &["id"]
            ))
            .is_err()
        );
        read::binds_in_order(
            "SELECT 1 WHERE a::text = lower($1) AND b::text = $2",
            &["a", "b"],
        );
        let extra = query.replace("plannedOn\n", "plannedOn\n    position\n");
        assert!(
            std::panic::catch_unwind(|| read::query_file_matches(sql, &extra, &["id"])).is_err()
        );
        assert!(
            std::panic::catch_unwind(|| read::query_file_matches(sql, query, &["session"]))
                .is_err()
        );
    }
}
