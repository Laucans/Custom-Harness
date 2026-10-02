//! Proof, not unit test: `harness-workflows` can indeed see
//! `harness-core` — the direction that the `ALLOWED` table in `test_layering.py`
//! verified on the Python side. Here, a `use` in the other direction simply
//! would not compile; this test documents the direction that works.

#[test]
fn workflows_can_reach_into_core() {
    let halt = harness_core::domain::Halt::Halted("test".into());
    assert_eq!(halt.exit_code(), 1);
}
