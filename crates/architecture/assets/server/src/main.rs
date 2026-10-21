//! The product's server: the shell's wire protocol through the DataGuard
//! engine, planted with every unit of the registry.
//!
//!   server migrate          creates the engine's own tables
//!   server serve [addr]     serves; addr, else 127.0.0.1:$PORT, else 127.0.0.1:8800
//!   server inventory        lists the registered units, with what each needs
//!
//! `DATABASE_URL` names the database, or is `memory`: the engine's
//! in-memory store, which forgets everything at exit. `inventory` reads no
//! database.
//!
//! Installed once by `harness init-repo`, then the product's: what it shares
//! with its units (`shared`), its own routes beside `/health`, its own
//! subcommands. The generic part — the engine, the routes, the applier — is
//! `dataguard_server`'s, and a unit is registered by its folder and
//! `scripts/register.sh`, never here.

use std::sync::Arc;

use axum::routing::get;
use dataguard_server::engine::plugins::Shared;
use dataguard_server::{Failure, applier, migrate, plant, registry, router};

/// What the product shares with its units: one value per type a unit
/// declares it `needs`. The server refuses to start while a need has
/// nobody to fill it.
fn shared() -> Shared {
    // let mut shared = Shared::default();
    // shared.insert(Arc::new(my_product::Llm::from_env()));
    Shared::default()
}

fn database_url() -> Result<String, Failure> {
    std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must name the database, or be `memory`".into())
}

/// Every unit the registry holds, one per line: its kind, its folder, and
/// the types it reads from `Shared`.
fn inventory() -> String {
    let mut out = String::new();
    for (kind, units) in [
        ("aggregate", registry::AGGREGATES),
        ("data-capability", registry::DATA_CAPABILITIES),
        ("capability", registry::CAPABILITIES),
    ] {
        for unit in units {
            let needs = unit.needs();
            let needs = if needs.is_empty() {
                "-".to_string()
            } else {
                needs.join(", ")
            };
            let code = if unit.plugin.is_some() {
                ""
            } else {
                " (manifest only)"
            };
            out.push_str(&format!(
                "{kind:<16} {}{code}  needs: {needs}\n",
                unit.folder
            ));
        }
    }
    out
}

async fn serve(addr: String) -> Result<(), Failure> {
    let planted = plant(&database_url()?, shared()).await?;
    let app = router(&planted).route("/health", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let applier = applier(Arc::clone(&planted.engine));
    println!("server on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    applier.abort();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("migrate") => {
            println!("{}", migrate(&database_url()?).await?);
            Ok(())
        }
        Some("inventory") => {
            print!("{}", inventory());
            Ok(())
        }
        Some("serve") | None => {
            let addr = args.next().unwrap_or_else(|| {
                let port = std::env::var("PORT").unwrap_or_else(|_| "8800".to_string());
                format!("127.0.0.1:{port}")
            });
            serve(addr).await
        }
        Some(other) => {
            Err(format!("unknown command {other}: migrate | serve [addr] | inventory").into())
        }
    }
}
