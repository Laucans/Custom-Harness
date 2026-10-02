//! Ce que le raffinage lit et décide : son état, le compteur de rounds, les
//! cinq sections du corps.
//!
//! Métier pur — rien ici n'appelle `gh` ni n'écrit nulle part. Ce qui écrit
//! vit dans `action`, ce qui juge dans `checks`.

pub mod rounds;
pub mod sections;
pub mod state;
