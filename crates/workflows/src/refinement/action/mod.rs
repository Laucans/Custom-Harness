//! Ce que le raffinage **fait** : demander une étape, consigner ce que le
//! routeur a nommé, réécrire le corps de l'issue.
//!
//! Le pendant de `checks` : une `Verification` juge et n'écrit pas, donc tout
//! ce qui écrit vit ici — y compris la moitié « écrit » de l'ancienne
//! `router_named_sections`.

pub mod actions;
pub mod publish;
