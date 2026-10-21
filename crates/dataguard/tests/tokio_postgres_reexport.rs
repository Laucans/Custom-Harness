//! An aggregate crate's `ProjectPg`, written as a product writes it: the
//! PostgreSQL types through `harness_dataguard::tokio_postgres`, no
//! `tokio-postgres` of its own. It compiles only if the re-export is the
//! engine's own client, so one `Transaction` type.
#![cfg(feature = "postgres")]

use async_trait::async_trait;
use harness_dataguard::plugins::{Plugin, Plugins, ProjectPg, ProjectionError};
use harness_dataguard::store::Write;
use harness_dataguard::tokio_postgres::{Client, Transaction};

struct Ledger;

#[async_trait]
impl ProjectPg for Ledger {
    async fn project(&self, write: &Write, tx: &Transaction<'_>) -> Result<(), ProjectionError> {
        tx.execute("select $1::text", &[&write.partition]).await?;
        Ok(())
    }

    async fn warm(&self, client: &Client) -> Result<(), ProjectionError> {
        client.batch_execute("select 1").await?;
        Ok(())
    }
}

#[test]
fn a_projection_written_over_the_re_export_plugs_in() {
    let plugins: Plugins = std::iter::once(Plugin::new().project_pg("Account", Ledger)).collect();
    assert!(plugins.needs().is_empty());
}
