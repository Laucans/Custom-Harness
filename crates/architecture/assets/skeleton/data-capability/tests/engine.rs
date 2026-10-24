//! `{{id}}` as the server plants it beside {{Agg}}, over the in-memory
//! store (`platform_test_kit::Bench`). SCAFFOLD: what these tests prove, one
//! clause per acceptance criterion, each invariant of the manifest named by
//! the test that breaks it.

use platform_test_kit::{Bench, Served, file};

/// The engine as the server plants this unit and its aggregate.
async fn served() -> Served {
    Bench::new(
        include_str!("../data-capability.json"),
        &file("{{agg_dir_root}}/aggregate.json"),
        [{{agg_lib}}::plugin(), {{lib}}::plugin()],
    )
    // SCAFFOLD: the test-only DataCapabilities planting the rows it reads:
    // `.planting(&platform_test_kit::planter("test.<verb>", "<Aggregate>",
    // "$<idField>", "insert"))`, then `served.plant(..)` in the tests.
    .memory()
    .await
}

#[tokio::test]
async fn the_unit_is_planted_beside_its_aggregate() {
    let served = served().await;
    assert_eq!(served.settled().await.len(), 0, "nothing queued yet");
}

#[tokio::test]
async fn scaffold_one_test_per_acceptance_criterion() {
    let _served = served().await;
    panic!("SCAFFOLD: one test per acceptance criterion of the SPEC, through served.applied(..)");
}
