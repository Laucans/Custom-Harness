//! Ce que la revue lit et décide : son état, les règles de saut, le texte
//! qu'elle publie.
//!
//! Métier pur — rien ici n'appelle `gh` ni n'écrit nulle part. Ce qui écrit
//! vit dans `action`, ce qui juge dans `checks`.

pub mod findings;
pub mod notes;
pub mod skip_rules;
pub mod state;
