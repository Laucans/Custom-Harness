//! Les étiquettes `harness:*`, partagées par les workflows qui les lisent.
//!
//! Elles sont **créées à la main sur le dépôt**, et le préflight de chaque
//! workflow vérifie que les siennes existent avant de payer quoi que ce soit :
//! une étiquette mal orthographiée rend le tableau vide, et un tableau vide se
//! lit « plus rien à faire ».
//!
//! Renommées depuis `pipeline:*` — la migration prend la main sur le suivi
//! plutôt que de cohabiter. Le renommage côté GitHub est un geste humain : il
//! arrête le pipeline Python à la seconde où il est fait. `REFINEMENT` est
//! renommée dès l'écriture, comme les sept autres — ce qui reste humain est le
//! `gh label edit` lui-même, documenté dans `docs/CUTOVER.md`.

/// Un item de roadmap : ce dont `/planner` tire un milestone.
pub const ROADMAP: &str = "harness:roadmap";

/// Un milestone : le lot de travail en cours.
pub const MILESTONE: &str = "harness:milestone";

/// Une task que l'agent peut faire tourner.
pub const AGENT: &str = "harness:agent";

/// Ce que seul l'humain peut faire. Bloque par dépendance, pas par mécanisme.
pub const HUMAN: &str = "harness:human";

/// La case que seul l'humain coche. Rien n'est pris sans elle.
pub const READY: &str = "harness:ready";

/// Le SPEC a été écrit dans le corps de l'issue.
pub const SPEC_WRITTEN: &str = "harness:spec-written";

/// Livrée sur la branche d'intégration, pas encore fusionnée dans `main`.
///
/// L'issue reste **ouverte** : la fermer dirait que le travail est intégré, ce
/// qui n'est vrai qu'après la fusion. Ce troisième état est ce qui sépare
/// « l'agent a fini » de « c'est dans `main` ».
pub const WAITING_MERGE: &str = "harness:waiting-merge";

/// Un round de raffinage reste à faire sur cette issue.
///
/// Posée par un humain (ou le planner) pour demander un round ; le raffinage
/// la retire lui-même une fois le round écrit.
pub const REFINEMENT: &str = "harness:refinement";

/// Les sept que la boucle exige.
///
/// `REFINEMENT` n'en fait pas partie : la boucle ne la lit pas, et son
/// préflight refuserait de tourner sans elle.
pub const LOOP: [&str; 7] = [
    ROADMAP,
    MILESTONE,
    AGENT,
    HUMAN,
    READY,
    SPEC_WRITTEN,
    WAITING_MERGE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_the_loop_requires_is_in_the_list() {
        // Le préflight lit cette liste : une étiquette oubliée ici serait une
        // étiquette qu'il ne vérifie pas, donc un tableau vide non expliqué.
        for label in [
            ROADMAP,
            MILESTONE,
            AGENT,
            HUMAN,
            READY,
            SPEC_WRITTEN,
            WAITING_MERGE,
        ] {
            assert!(LOOP.contains(&label), "{label} manque à LOOP");
        }
    }

    #[test]
    fn they_all_share_the_namespace_and_none_is_a_prefix_of_another() {
        let all = [
            ROADMAP,
            MILESTONE,
            AGENT,
            HUMAN,
            READY,
            SPEC_WRITTEN,
            WAITING_MERGE,
            REFINEMENT,
        ];
        for label in all {
            assert!(label.starts_with("harness:"), "{label} hors du namespace");
        }
        // Une étiquette préfixe d'une autre rendrait un `has()` ambigu si
        // quelqu'un passait un jour à une comparaison par préfixe.
        for a in all {
            let prefixes = all.iter().filter(|b| b.starts_with(a)).count();
            assert_eq!(prefixes, 1, "{a} est le préfixe d'une autre étiquette");
        }
    }
}
