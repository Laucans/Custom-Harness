//! `{{id}}` over PostgreSQL (`DATABASE_URL`), as the server plants it beside
//! {{Agg}}, whose projection writes its tables: each test works in a
//! scratch database of its own, migrated as the server migrates it, dropped
//! when it ends (`platform_test_kit::Scratch`). SCAFFOLD: what the tables
//! hold after the command.

use platform_test_kit::{Bench, Scratch, Served, file};

/// The engine as the server plants this unit and its aggregate over
/// `scratch`'s tables.
async fn served(scratch: &Scratch) -> Served {
    Bench::new(
        include_str!("../data-capability.json"),
        &file("{{agg_dir_root}}/aggregate.json"),
        [{{agg_lib}}::plugin(), {{lib}}::plugin()],
    )
    .postgres(scratch)
    .await
}

#[tokio::test]
async fn scaffold_what_the_tables_hold_after_the_command() {
    let scratch = Scratch::new("{{lib}}_tables").await;
    // SCAFFOLD: `scratch.seed("INSERT …")`, the rows the tables' foreign
    // keys ask for, in data/model.json's order.
    let _served = served(&scratch).await;
    panic!("SCAFFOLD: submit, settle, then read the tables with scratch.rows(..)");
}
