//! The product's DataGuard server, generic: the engine of `harness-dataguard`
//! planted with every unit the registry gathers (`dataguard_registry`), over
//! the product's PostgreSQL, speaking the wire protocol the shell client and
//! the mock already speak.
//!
//! - [`plant`]: the engine and the reads, from the registry's manifests and
//!   plugins and what the product shares with its units;
//! - [`router`]: the routes of the wire protocol;
//! - [`woken_applier`] and [`waking`]: the single applier, every second
//!   and at once when a write reaches the wire; [`applier`] is the same
//!   applier on its clock alone, for a binary that wakes nothing;
//! - [`migrate`]: the engine's own tables.
//!
//! The product's binary (`crates/server`) composes them, and is where the
//! product grows: what it shares, its own routes, its subcommands. This
//! crate holds no unit and names none: a unit is a folder the registry
//! finds, registered by `scripts/register.sh`.
//!
//! `DATABASE_URL` names the database, or is [`MEMORY`]: the engine's
//! in-memory store, which forgets everything at exit, for a mock-up, a demo,
//! a smoke test.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use harness_dataguard::Error;
use harness_dataguard::aggregate::{Aggregate, duration_seconds};
use harness_dataguard::api::{
    HoldRequest, ReadRequest, ReadResponse, ReviewDecision, Submission, Submitted,
};
use harness_dataguard::engine::Engine;
use harness_dataguard::manifest::DataCapability;
use harness_dataguard::memory::{MemoryNotifier, MemoryStore};
use harness_dataguard::plugins::{RefusalKind, Shared};
use harness_dataguard::postgres::{MIGRATION, PgStore, ReadOnly, SystemClock};
use harness_dataguard::queue::Hold;
use harness_dataguard::reads::{Reads, Rows};
use harness_dataguard::registry::Registry;
use harness_dataguard::store::{RandomIds, Store};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

pub use dataguard_registry as registry;
pub use harness_dataguard as engine;

/// The `DATABASE_URL` that means no database: the in-memory store.
pub const MEMORY: &str = "memory";

/// Why the server did not start.
pub type Failure = Box<dyn std::error::Error + Send + Sync>;

/// The engine and the reads, planted: what the routes serve.
#[derive(Clone)]
pub struct Planted {
    /// The engine, over the store and every unit's plugin.
    pub engine: Arc<Engine>,
    /// The Capabilities' reads.
    pub reads: Arc<Reads>,
}

/// A client on `url`, its connection driven in the background.
async fn connect(url: &str) -> Result<tokio_postgres::Client, Failure> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("postgres: {e}");
        }
    });
    Ok(client)
}

/// The engine over this product's units and its database, with what the
/// product shares with its units.
///
/// Every aggregate and `DataCapability` manifest the registry found is
/// registered, every unit's plugin is plugged in, and the engine's `load()`
/// refuses units that do not fit: a plugin for a unit it does not
/// register, a need nothing shares (`Error::Misconfigured`). Over
/// PostgreSQL the store runs the aggregates' projections in the apply
/// transaction (and hydrates from them a row the engine never wrote), and
/// the reads have a read-only connection of their own, which the
/// `Prepare`s read through too (`Context::rows`).
///
/// # Errors
///
/// The database does not answer, a manifest does not parse, or the units
/// do not fit together.
pub async fn plant(url: &str, shared: Shared) -> Result<Planted, Failure> {
    let plugins = registry::plugins();
    let mut reads = Reads::from(&plugins);
    // Over PostgreSQL, one read-only connection serves the Capabilities'
    // reads and the `Prepare`s' SQL (`Context::rows`); over memory, neither
    // has any.
    let mut read_only: Option<Arc<dyn Rows>> = None;
    let store: Arc<dyn Store> = if url == MEMORY {
        Arc::new(MemoryStore::default())
    } else {
        let rows: Arc<dyn Rows> = Arc::new(ReadOnly::new(connect(url).await?));
        reads = reads.with_connection(rows.clone());
        read_only = Some(rows);
        Arc::new(
            PgStore::open(connect(url).await?)
                .await?
                .with_projections(&plugins)
                .await?,
        )
    };
    let mut engine = Engine::new(
        store,
        Arc::new(MemoryNotifier::default()),
        Arc::new(SystemClock),
    )
    .with_registry(Registry::parse(registry::QUERIES)?)
    .with_ids(Arc::new(RandomIds));
    if let Some(rows) = read_only {
        engine = engine.with_rows(rows);
    }
    for text in registry::AGGREGATES.iter().filter_map(|unit| unit.manifest) {
        engine = engine.with_aggregate(Aggregate::parse(text)?);
    }
    for text in registry::DATA_CAPABILITIES
        .iter()
        .filter_map(|unit| unit.manifest)
    {
        engine = engine.with_data_capability(DataCapability::parse(text)?);
    }
    // engine = engine.with_proposer(Arc::new(my_product::Proposer));
    let engine = engine.with_plugins(plugins).with_shared(shared);
    engine.load().await?;
    Ok(Planted {
        engine: Arc::new(engine),
        reads: Arc::new(reads),
    })
}

/// The engine's own tables, created; nothing over the in-memory store.
/// What happened, for the operator.
///
/// # Errors
///
/// The database does not answer or refuses the migration.
pub async fn migrate(url: &str) -> Result<&'static str, Failure> {
    if url == MEMORY {
        return Ok("nothing to migrate: the in-memory store");
    }
    connect(url).await?.batch_execute(MIGRATION).await?;
    Ok("migrated")
}

/// How long the applier waits when no write wakes it: the holds' and the
/// parked commands' expiries are seen within it.
const TICK: Duration = Duration::from_secs(1);

/// The routes that read and never write: no reason to wake the applier.
const READS: &str = "/capabilities/";

/// The single applier on its clock alone: every second, each partition's
/// head. Its handle is the caller's, to abort on shutdown. A command waits
/// for the next tick before it is applied; [`woken_applier`] with
/// [`waking`] applies it at once.
#[must_use]
pub fn applier(engine: Arc<Engine>) -> JoinHandle<()> {
    woken_applier(engine, Arc::new(Notify::new()))
}

/// The single applier: every second, and as soon as `wake` is notified —
/// by [`waking`], once each write the wire answers is answered — so what
/// one tab does shows in the other within the second. Its handle is the
/// caller's, to abort on shutdown.
#[must_use]
pub fn woken_applier(engine: Arc<Engine>, wake: Arc<Notify>) -> JoinHandle<()> {
    driving(wake, TICK, move || {
        let engine = Arc::clone(&engine);
        async move {
            if let Err(e) = engine.run_once().await {
                eprintln!("applier: {e}");
            }
        }
    })
}

/// `run` once per `tick`, and once more each time `wake` is notified.
fn driving<F, R>(wake: Arc<Notify>, tick: Duration, run: R) -> JoinHandle<()>
where
    R: Fn() -> F + Send + 'static,
    F: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = tokio::time::sleep(tick) => {}
                () = wake.notified() => {}
            }
            run().await;
        }
    })
}

/// `app` notifying `wake` once each write it answers — a command
/// submitted, confirmed, cancelled or requeued, a hold or a review — has
/// been answered, its command queued. Wrap the whole app, the product's
/// own routes with [`router`]'s.
pub fn waking(app: Router, wake: Arc<Notify>) -> Router {
    app.layer(middleware::from_fn_with_state(wake, woken))
}

/// Whether a request writes: a `POST` anywhere but a Capability's read.
fn writes(method: &Method, path: &str) -> bool {
    method == Method::POST && !path.starts_with(READS)
}

/// One request through [`waking`]: answered, then the applier woken when it
/// was a write.
async fn woken(State(wake): State<Arc<Notify>>, request: Request, next: Next) -> Response {
    let write = writes(request.method(), request.uri().path());
    let response = next.run(request).await;
    if write {
        wake.notify_one();
    }
    response
}

/// The routes of the wire protocol, over what [`plant`] planted.
pub fn router(planted: &Planted) -> Router {
    Router::new()
        .route("/capabilities/{id}", post(capability))
        .route("/commands", post(submit))
        .route("/commands/{id}", get(status))
        .route("/commands/{id}/confirm", post(confirm))
        .route("/commands/{id}/cancel", post(cancel))
        .route("/commands/{id}/requeue", post(requeue))
        .route("/holds", post(hold))
        .route("/holds/{id}/renew", post(renew))
        .route("/holds/{id}/release", post(release))
        .route("/reviews/{id}", post(decide))
        .route("/dataVersion", get(version))
        .with_state(planted.clone())
}

/// The HTTP status an engine error answers with: a refusal of the
/// product's code is 404 when it names nothing that exists, 422 otherwise.
const fn status_of(error: &Error) -> StatusCode {
    match error {
        Error::NotFound(_) | Error::UnknownDataCapability(_) | Error::UnregisteredQuery(_) => {
            StatusCode::NOT_FOUND
        }
        Error::Forbidden(_) => StatusCode::FORBIDDEN,
        Error::NotNow(_) => StatusCode::CONFLICT,
        Error::Store(_) => StatusCode::SERVICE_UNAVAILABLE,
        Error::Misconfigured(_) => StatusCode::INTERNAL_SERVER_ERROR,
        Error::Refused(refusal) => match refusal.kind {
            RefusalKind::NotFound => StatusCode::NOT_FOUND,
            RefusalKind::Unprocessable => StatusCode::UNPROCESSABLE_ENTITY,
        },
    }
}

type Reply<T> = Result<(StatusCode, Json<T>), (StatusCode, Json<Value>)>;

fn reply<T>(status: StatusCode, result: Result<T, Error>) -> Reply<T> {
    result
        .map(|value| (status, Json(value)))
        .map_err(|e| (status_of(&e), Json(json!({ "error": e.to_string() }))))
}

/// A submission, its row named by `targetId` or `"target": {"id": …}` (the
/// engine's `Submission` reads both); the follow-ups its `Prepare` asked
/// for come back in `then`, each submitted or refused.
async fn submit(State(planted): State<Planted>, Json(body): Json<Submission>) -> Reply<Submitted> {
    reply(StatusCode::ACCEPTED, planted.engine.submit(body).await)
}

async fn status(State(planted): State<Planted>, Path(id): Path<String>) -> Reply<Value> {
    reply(
        StatusCode::OK,
        planted
            .engine
            .status(&id)
            .await
            .map(|r| json!({ "result": r })),
    )
}

#[derive(serde::Deserialize)]
struct By {
    by: String,
}

async fn confirm(
    State(planted): State<Planted>,
    Path(id): Path<String>,
    Json(who): Json<By>,
) -> Reply<Value> {
    reply(
        StatusCode::OK,
        planted
            .engine
            .confirm(&id, &who.by)
            .await
            .map(|r| json!({ "result": r })),
    )
}

async fn cancel(
    State(planted): State<Planted>,
    Path(id): Path<String>,
    Json(who): Json<By>,
) -> Reply<Value> {
    reply(
        StatusCode::OK,
        planted
            .engine
            .cancel(&id, &who.by)
            .await
            .map(|r| json!({ "result": r })),
    )
}

async fn requeue(
    State(planted): State<Planted>,
    Path(id): Path<String>,
    Json(who): Json<By>,
) -> Reply<Submitted> {
    reply(
        StatusCode::ACCEPTED,
        planted.engine.requeue(&id, &who.by).await,
    )
}

async fn hold(State(planted): State<Planted>, Json(body): Json<HoldRequest>) -> Reply<Hold> {
    let ttl = body.ttl.as_deref().and_then(duration_seconds);
    reply(
        StatusCode::CREATED,
        planted.engine.hold(&body.partition, &body.by, ttl).await,
    )
}

async fn renew(State(planted): State<Planted>, Path(id): Path<String>) -> Reply<Hold> {
    reply(StatusCode::OK, planted.engine.renew(&id).await)
}

async fn release(State(planted): State<Planted>, Path(id): Path<String>) -> Reply<Value> {
    reply(
        StatusCode::OK,
        planted.engine.release(&id).await.map(|()| json!({})),
    )
}

async fn decide(
    State(planted): State<Planted>,
    Path(id): Path<String>,
    Json(body): Json<ReviewDecision>,
) -> Reply<Value> {
    reply(
        StatusCode::OK,
        planted
            .engine
            .decide(&id, &body.by, &body.roles, body.approved)
            .await
            .map(|r| json!({ "result": r })),
    )
}

/// A Capability's read: its plugin's query and answer (`Reads::run`). A
/// request naming a persisted query's hash is served only when the
/// registry holds it under this Capability (contract C). `asOf` is read
/// before the rows, so the rows are at least that recent.
async fn capability(
    State(planted): State<Planted>,
    Path(id): Path<String>,
    Json(body): Json<ReadRequest>,
) -> Reply<ReadResponse> {
    let answer = async {
        if let Some(sha256) = body.sha256.as_deref() {
            planted.engine.served(&id, sha256)?;
        }
        let as_of = planted.engine.data_version().await?;
        let data = planted.reads.run(&id, &body.variables).await?;
        Ok(ReadResponse { as_of, data })
    };
    reply(StatusCode::OK, answer.await)
}

async fn version(State(planted): State<Planted>) -> Reply<Value> {
    reply(
        StatusCode::OK,
        planted
            .engine
            .data_version()
            .await
            .map(|v| json!({ "dataVersion": v })),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A counter `driving` adds one to at each run.
    fn counting(runs: &Arc<AtomicUsize>) -> impl Fn() -> std::future::Ready<()> + Send + 'static {
        let counted = Arc::clone(runs);
        move || {
            counted.fetch_add(1, Ordering::SeqCst);
            std::future::ready(())
        }
    }

    #[test]
    fn a_write_wakes_the_applier_and_a_read_does_not() {
        assert!(writes(&Method::POST, "/commands"));
        assert!(writes(&Method::POST, "/commands/c1/confirm"));
        assert!(writes(&Method::POST, "/holds"));
        assert!(!writes(&Method::POST, "/capabilities/table.listTables"));
        assert!(!writes(&Method::GET, "/commands/c1"));
        assert!(!writes(&Method::GET, "/dataVersion"));
    }

    #[tokio::test]
    async fn the_applier_runs_when_woken_without_waiting_for_its_tick() {
        let wake = Arc::new(Notify::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let handle = driving(
            Arc::clone(&wake),
            Duration::from_secs(3600),
            counting(&runs),
        );
        wake.notify_one();
        let ran = tokio::time::timeout(Duration::from_secs(1), async {
            while runs.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        handle.abort();
        assert!(ran.is_ok(), "the applier did not run once woken");
    }

    #[tokio::test]
    async fn the_applier_runs_every_tick_unwoken() {
        let runs = Arc::new(AtomicUsize::new(0));
        let handle = driving(
            Arc::new(Notify::new()),
            Duration::from_millis(10),
            counting(&runs),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();
        assert!(runs.load(Ordering::SeqCst) >= 2);
    }
}
