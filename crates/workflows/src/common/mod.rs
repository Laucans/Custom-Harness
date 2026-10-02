//! Ce que plusieurs workflows lisent, écrit une fois.
//!
//! N'y vont que des choses dont **au moins deux** workflows ont réellement
//! besoin, aujourd'hui — pas ce qui pourrait un jour servir à un troisième.
//! `labels` sert la boucle et le raffinage ; `explore` sert le raffinage, et
//! sa place ici tient pour le jour où un troisième workflow veut la même
//! carte du dépôt, pas parce qu'il existe déjà.

pub mod labels;

#[cfg(test)]
pub(crate) mod fake_github;
