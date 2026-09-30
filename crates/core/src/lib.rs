#![warn(clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![deny(unsafe_code)]
// La seule exception à « pas d'`allow` à la racine » (`CLAUDE.md`), et elle est
// catégorique plutôt que locale : `clippy::nursery` inclut `future_not_send`,
// qui présuppose qu'on veut des futurs `Send`. La décision n°3 de
// `docs/MIGRATION.md` dit le contraire — `#[async_trait(?Send)]` partout,
// tokio `current_thread`, parce que le harness pilote une session à la fois et
// n'a aucune borne `Send` à payer. Le futur de chaque méthode de trait est donc
// non-`Send` par construction, et tout ce qui l'attend l'est aussi : le lint
// tirerait sur presque chaque `async fn` du paquet. Le paperasser site par site
// dirait « défaut connu » là où il n'y a qu'un désaccord avec un choix assumé.
#![allow(clippy::future_not_send)]

//! Le framework du harness : vocabulaire, traces, exécution.
//!
//! Rien ici n'importe `harness-workflows` ni `harness-launcher` — et Cargo
//! refuse de compiler l'inverse. L'invariant « le framework ignore ses
//! utilisateurs » est donc tenu par le graphe de crates, à la compilation,
//! plutôt que par un test qui parcourt des AST.

pub mod adapters;
pub mod domain;
pub mod execution;
pub mod traces;
