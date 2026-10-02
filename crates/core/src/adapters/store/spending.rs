//! Où une dépense est consignée — le port, pas le fichier.
//!
//! [`Ledger`](super::ledger::Ledger) sait écrire une ligne, mais pas quelle
//! heure il est, ni comment s'appelle ce run, ni sur quelle machine il tourne.
//! Ces trois-là sont des faits du lanceur, pas de l'exécution : une action qui
//! vient de payer une session sait ce qu'elle a dépensé et pour quelle task,
//! et rien de plus.
//!
//! D'où ce port. L'implémentation vit dans `harness-launcher`, qui est le seul
//! à tenir une horloge et un nom de machine — et c'est aussi ce qui garde
//! `harness-core` sans dépendance au temps.

use crate::domain::{Outcome, Spend};

/// Ce qu'une stage finie a coûté, tel que l'exécution le sait.
///
/// Emprunté plutôt que possédé : la ligne est écrite dans l'appel, rien n'est
/// gardé après.
pub struct Entry<'a> {
    /// Le numéro de round.
    pub round: u32,
    /// La task facturée. Vide sur un round de rollover, qui n'en a pas.
    pub task: &'a str,
    /// La stage.
    pub stage: &'a str,
    /// Ce que le porteur de session a observé — champs non observés compris.
    pub spend: &'a Spend,
    /// `ok`, ou la raison de l'absence de réponse.
    pub outcome: &'a str,
}

/// Où une dépense est consignée.
pub trait Spending {
    /// Consigne ce qu'une stage a coûté.
    ///
    /// # Errors
    ///
    /// L'échec de l'écriture, et il **arrête le round** : le budget d'un run
    /// se lit dans le registre, et une ligne perdue le fait mentir sur une
    /// dépense déjà engagée.
    fn record(&self, entry: &Entry<'_>) -> Outcome<()>;
}
