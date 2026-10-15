//! The product's DataGuard server: the engine of `harness-dataguard` bound
//! to this product's aggregates and DataCapabilities, over its PostgreSQL,
//! speaking the wire protocol the shell client and the mock already speak.
//!
//!   dataguard-server migrate          applies the engine's own migration
//!   dataguard-server serve [addr]     serves, 127.0.0.1:8800 by default
//!
//! `DATABASE_URL` names the database. The invariants a product executes
//! beyond `aggregate.json`'s rules, and the AI that proposes
//! reconciliations, are wired in `plant()` — the two places this file
//! expects the product to grow.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use harness_dataguard::aggregate::Aggregate;
use harness_dataguard::api::{HoldRequest, ReadRequest, ReadResponse, ReviewDecision, Submission};
use harness_dataguard::engine::Engine;
use harness_dataguard::manifest::DataCapability;
use harness_dataguard::memory::MemoryNotifier;
use harness_dataguard::postgres::{PgStore, SystemClock};
use harness_dataguard::registry::Registry;
use harness_dataguard::Error;

mod registry {
    include!(concat!(env!("OUT_DIR"), "/registry.rs"));
}

type Shared = Arc<Engine>;

/// The engine over this product's manifests and its database.
async fn plant(url: &str) -> Result<Engine, Box<dyn std::error::Error>> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("postgres: {e}");
        }
    });
    let store = Arc::new(PgStore::open(client).await?);
    let mut engine = Engine::new(store, Arc::new(MemoryNotifier::default()), Arc::new(SystemClock))
        .with_registry(Registry::parse(registry::REGISTRY)?);
    for text in registry::AGGREGATES {
        engine = engine.with_aggregate(Aggregate::parse(text)?);
    }
    for text in registry::DATA_CAPABILITIES {
        engine = engine.with_data_capability(DataCapability::parse(text)?);
    }
    // engine = engine.with_invariants(Arc::new(my_product::Invariants));
    // engine = engine.with_proposer(Arc::new(my_product::Proposer));
    engine.load().await?;
    Ok(engine)
}

fn status_of(error: &Error) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    match error {
        Error::NotFound(_) | Error::UnknownDataCapability(_) | Error::UnregisteredQuery(_) => StatusCode::NOT_FOUND,
        Error::Forbidden(_) => StatusCode::FORBIDDEN,
        Error::NotNow(_) => StatusCode::CONFLICT,
        Error::Store(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

type Reply<T> = Result<(axum::http::StatusCode, Json<T>), (axum::http::StatusCode, Json<serde_json::Value>)>;

fn reply<T>(status: axum::http::StatusCode, result: Result<T, Error>) -> Reply<T> {
    result
        .map(|value| (status, Json(value)))
        .map_err(|e| (status_of(&e), Json(serde_json::json!({ "error": e.to_string() }))))
}

async fn submit(State(engine): State<Shared>, Json(body): Json<Submission>) -> Reply<harness_dataguard::api::Submitted> {
    reply(axum::http::StatusCode::ACCEPTED, engine.submit(body).await)
}

async fn status(State(engine): State<Shared>, Path(id): Path<String>) -> Reply<serde_json::Value> {
    reply(
        axum::http::StatusCode::OK,
        engine.status(&id).await.map(|r| serde_json::json!({ "result": r })),
    )
}

#[derive(serde::Deserialize)]
struct By {
    by: String,
}

async fn confirm(State(engine): State<Shared>, Path(id): Path<String>, Json(who): Json<By>) -> Reply<serde_json::Value> {
    reply(axum::http::StatusCode::OK, engine.confirm(&id, &who.by).await.map(|r| serde_json::json!({ "result": r })))
}

async fn cancel(State(engine): State<Shared>, Path(id): Path<String>, Json(who): Json<By>) -> Reply<serde_json::Value> {
    reply(axum::http::StatusCode::OK, engine.cancel(&id, &who.by).await.map(|r| serde_json::json!({ "result": r })))
}

async fn requeue(State(engine): State<Shared>, Path(id): Path<String>, Json(who): Json<By>) -> Reply<harness_dataguard::api::Submitted> {
    reply(axum::http::StatusCode::ACCEPTED, engine.requeue(&id, &who.by).await)
}

async fn hold(State(engine): State<Shared>, Json(body): Json<HoldRequest>) -> Reply<harness_dataguard::queue::Hold> {
    let ttl = body.ttl.as_deref().and_then(harness_dataguard::aggregate::duration_seconds);
    reply(axum::http::StatusCode::CREATED, engine.hold(&body.partition, &body.by, ttl).await)
}

async fn renew(State(engine): State<Shared>, Path(id): Path<String>) -> Reply<harness_dataguard::queue::Hold> {
    reply(axum::http::StatusCode::OK, engine.renew(&id).await)
}

async fn release(State(engine): State<Shared>, Path(id): Path<String>) -> Reply<serde_json::Value> {
    reply(axum::http::StatusCode::OK, engine.release(&id).await.map(|()| serde_json::json!({})))
}

async fn decide(State(engine): State<Shared>, Path(id): Path<String>, Json(body): Json<ReviewDecision>) -> Reply<serde_json::Value> {
    reply(
        axum::http::StatusCode::OK,
        engine.decide(&id, &body.by, &body.roles, body.approved).await.map(|r| serde_json::json!({ "result": r })),
    )
}

/// A read through a persisted query: served only when the registry holds
/// the Capability's query under this hash (contract C). What the query
/// answers is the product's read crate's — `read()` is where it plugs in.
async fn capability(State(engine): State<Shared>, Path(id): Path<String>, Json(body): Json<ReadRequest>) -> Reply<ReadResponse> {
    let served = engine.served(&id, &body.sha256);
    let answer = match served {
        Ok(()) => engine.data_version().await.map(|as_of| ReadResponse {
            as_of,
            data: read(&id, &body.variables),
        }),
        Err(e) => Err(e),
    };
    reply(axum::http::StatusCode::OK, answer)
}

/// The product's read crate answers here: `<system>_read::run(capability, variables)`.
fn read(_capability: &str, _variables: &std::collections::BTreeMap<String, serde_json::Value>) -> serde_json::Value {
    serde_json::Value::Null
}

async fn version(State(engine): State<Shared>) -> Reply<serde_json::Value> {
    reply(axum::http::StatusCode::OK, engine.data_version().await.map(|v| serde_json::json!({ "dataVersion": v })))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must name the database")?;
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("migrate") => {
            let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await?;
            tokio::spawn(connection);
            client.batch_execute(harness_dataguard::postgres::MIGRATION).await?;
            println!("migrated");
            Ok(())
        }
        Some("serve") | None => {
            let engine = Arc::new(plant(&url).await?);
            let addr = args.next().unwrap_or_else(|| "127.0.0.1:8800".to_string());
            let app = Router::new()
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
                .with_state(engine.clone());
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            // The single applier: every second, each partition's head.
            let applier = tokio::spawn({
                let engine = engine.clone();
                async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        if let Err(e) = engine.run_once().await {
                            eprintln!("applier: {e}");
                        }
                    }
                }
            });
            println!("dataguard-server on {addr}");
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
            applier.abort();
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other}: migrate | serve [addr]").into()),
    }
}
