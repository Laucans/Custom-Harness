//! Preuve, pas test unitaire : `harness-workflows` voit bien
//! `harness-core` — le sens que la table `ALLOWED` de `test_layering.py`
//! vérifiait côté Python. Ici, un `use` dans l'autre sens ne compilerait
//! simplement pas ; ce test documente le sens qui, lui, fonctionne.

#[test]
fn workflows_can_reach_into_core() {
    let halt = harness_core::domain::Halt::Halted("test".into());
    assert_eq!(halt.exit_code(), 1);
}
