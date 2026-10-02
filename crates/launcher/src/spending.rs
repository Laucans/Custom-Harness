//! Le registre, câblé : l'horloge, l'identifiant du run, la machine.
//!
//! L'implémentation du port [`Spending`] vit ici et non dans `harness-core`,
//! et c'est délibéré. `Ledger` sait écrire une ligne mais pas quelle heure il
//! est ; une ligne de registre reçoit son instant plutôt que de le lire, ce qui
//! la rend testable. Les trois faits qui manquent — l'instant, le nom du run,
//! le nom de la machine — sont des faits du **lanceur**, et les mettre ici est
//! ce qui laisse `harness-core` sans aucune dépendance au temps.

use std::path::Path;

use harness_core::adapters::shell::process;
use harness_core::adapters::store::ledger::{Ledger, Row};
use harness_core::adapters::store::spending::{Entry, Spending};
use harness_core::domain::Outcome;

/// Le registre de ce run.
pub struct LedgerSpending {
    ledger: Ledger,
    run: String,
    host: String,
}

impl LedgerSpending {
    /// Le registre à ce chemin, pour ce run, sur cette machine.
    #[must_use]
    pub fn new(path: &Path, run: &str, host: &str) -> Self {
        Self {
            ledger: Ledger::new(path),
            run: run.to_string(),
            host: host.to_string(),
        }
    }
}

impl Spending for LedgerSpending {
    fn record(&self, entry: &Entry<'_>) -> Outcome<()> {
        self.ledger.append(&Row {
            when: now(),
            run: self.run.clone(),
            round: entry.round,
            task: entry.task.to_string(),
            stage: entry.stage.to_string(),
            spend: entry.spend.clone(),
            ran_on: self.host.clone(),
            outcome: entry.outcome.to_string(),
        })
    }
}

/// L'instant, en ISO-8601 UTC à la seconde.
///
/// En UTC là où le Python écrivait l'heure locale. Le **format** ne change pas,
/// donc l'en-tête gelé tient et les anciennes lignes restent lisibles ; seule la
/// valeur change, et un dépôt neuf n'a pas d'historique à contredire.
#[must_use]
pub fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Un identifiant de run : l'instant, compacté.
///
/// Il nomme le dossier de journal, la colonne `run` du registre et un workspace
/// jetable. Lisible à dessein — c'est ce qu'un humain tape pour retrouver les
/// traces d'un run.
#[must_use]
pub fn run_id() -> String {
    jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S").to_string()
}

/// Le nom de la machine, ou `?`.
///
/// Par le binaire `hostname` et l'adaptateur de processus : c'est un appel
/// externe, et les appels externes passent par `adapters`. Son absence n'arrête
/// rien — c'est une colonne de confort, pas une donnée dont dépend une
/// décision.
pub async fn hostname() -> String {
    let said = process::run("hostname", &[], Path::new(".")).await;
    match said {
        Ok(out) if out.ok() && !out.out().is_empty() => out.out().to_string(),
        _ => "?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::adapters::store::ledger::{self, HEADER};
    use harness_core::domain::{Spend, Tokens};

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("harness-spending-{tag}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("un dossier de test");
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_timestamp_has_the_shape_the_frozen_header_expects() {
        let stamp = now();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
    }

    #[test]
    fn a_run_id_is_readable_because_a_human_types_it_to_find_the_logs() {
        let id = run_id();
        assert_eq!(id.len(), 15, "{id}");
        assert_eq!(&id[8..9], "-");
    }

    #[test]
    fn a_recorded_spend_lands_on_the_frozen_columns() {
        let dir = Dir::new("columns");
        let path = dir.0.join("costs.tsv");
        let spending = LedgerSpending::new(&path, "20261002-120000", "le-mac");
        let spend = Spend {
            cost_usd: Some(1.5),
            turns: Some(7),
            tokens: Tokens {
                input: Some(1000),
                ..Tokens::default()
            },
            ..Spend::default()
        };
        spending
            .record(&Entry {
                round: 2,
                task: "11",
                stage: "code",
                spend: &spend,
                outcome: "ok",
            })
            .expect("écrit");
        let text = std::fs::read_to_string(&path).expect("relu");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(HEADER));
        let row: Vec<&str> = lines.next().expect("une ligne").split('\t').collect();
        let columns = ledger::columns();
        assert_eq!(row.len(), columns.len());
        let at = |name: &str| row[columns.iter().position(|c| *c == name).expect(name)];
        assert_eq!(at("run"), "20261002-120000");
        assert_eq!(
            at("round"),
            "02",
            "zéro-paddé, comme l'étiquette du journal"
        );
        assert_eq!(at("task"), "11");
        assert_eq!(at("stage"), "code");
        assert_eq!(at("cost_usd"), "1.500000");
        assert_eq!(at("ran_on"), "le-mac");
        assert_eq!(at("outcome"), "ok");
        // Non observé reste vide : une colonne de zéros se relirait comme une
        // session gratuite.
        assert_eq!(at("out"), "");
        assert_eq!(at("session"), "");
    }

    #[tokio::test]
    async fn a_hostname_that_cannot_be_read_does_not_stop_a_run() {
        // Une colonne de confort, pas une donnée dont dépend une décision.
        assert!(!hostname().await.is_empty());
    }
}
