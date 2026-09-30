//! Le registre de coûts : une ligne par stage, ce que le run a vraiment coûté.
//!
//! **L'en-tête est gelé.** Mêmes colonnes, même ordre que le registre Python,
//! et toute colonne nouvelle s'ajoute **en fin** de ligne : les anciennes
//! lignes en ont moins et restent lisibles telles quelles. C'est la seule
//! raison pour laquelle `when` est en UTC ici alors que le Python écrivait
//! l'heure locale — le format ne change pas, seule la valeur, et un dépôt neuf
//! n'a pas d'historique à contredire.
//!
//! Ce module **écrit et relit** le registre ; il ne le met pas en forme. Les
//! tables qu'un rapport affiche sont un autre métier.
//!
//! Le registre ne lit pas l'horloge : `when` arrive dans la ligne. Un registre
//! qui lirait l'heure lui-même ne serait pas testable, et c'est l'appelant qui
//! sait de quel instant il parle.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::domain::{Halt, Outcome, Spend};

/// L'en-tête, gelé caractère pour caractère.
pub const HEADER: &str = "when\trun\tround\ttask\tstage\tcost_usd\tturns\t\
                          duration_ms\tin\tout\tsession\tran_on\tcache_read\t\
                          cache_write\toutcome";

/// Les colonnes, dérivées de l'en-tête.
///
/// Une seule source de vérité pour l'ordre : étendre `HEADER` déplace les
/// colonnes avec lui, au lieu de laisser un compte tenu à la main dériver.
#[must_use]
pub fn columns() -> Vec<&'static str> {
    HEADER.split('\t').collect()
}

/// Ce qu'une stage a coûté, prêt à être écrit.
#[derive(Debug, Clone)]
pub struct Row {
    /// L'instant, en ISO-8601 à la seconde. Fourni, jamais lu d'une horloge.
    pub when: String,
    /// L'identifiant du run.
    pub run: String,
    /// Le numéro de round.
    pub round: u32,
    /// La task facturée.
    pub task: String,
    /// La stage.
    pub stage: String,
    /// Ce que le porteur de session a observé.
    pub spend: Spend,
    /// La machine qui a fait tourner ça.
    pub ran_on: String,
    /// `ok`, ou la raison de l'absence de réponse. Vide = non enregistré.
    pub outcome: String,
}

/// Un nombre observé, ou le vide.
///
/// Le vide et le zéro ne disent pas la même chose : `None` veut dire « non
/// observé », et l'écrire `0` ferait lire une session gratuite là où on n'a
/// rien su mesurer.
fn seen<T: ToString>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

/// Un texte libre, sans rien qui puisse casser une colonne.
///
/// Les tabulations et les retours à la ligne sont ce qui décale une ligne
/// entière d'une colonne — un titre de task en contient tôt ou tard.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Row {
    /// La ligne, telle qu'elle s'écrit.
    ///
    /// Assemblée dans l'ordre de [`columns`] : la ligne et l'en-tête ne peuvent
    /// pas dériver l'un de l'autre.
    #[must_use]
    pub fn render(&self) -> String {
        let cost = self
            .spend
            .cost_usd
            .map_or_else(String::new, |c| format!("{c:.6}"));
        let values = [
            self.when.clone(),
            flat(&self.run),
            format!("{:02}", self.round),
            flat(&self.task),
            flat(&self.stage),
            cost,
            seen(self.spend.turns),
            seen(self.spend.duration_ms),
            seen(self.spend.tokens.input),
            seen(self.spend.tokens.output),
            flat(self.spend.session.as_deref().unwrap_or_default()),
            flat(&self.ran_on),
            seen(self.spend.tokens.cache_read),
            seen(self.spend.tokens.cache_write),
            flat(&self.outcome),
        ];
        debug_assert_eq!(
            values.len(),
            columns().len(),
            "une ligne doit avoir exactement les colonnes de l'en-tête"
        );
        values.join("\t")
    }
}

/// Le registre, sur le disque.
pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    /// Le registre à ce chemin. Rien n'est créé avant la première écriture.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// Ajoute une ligne. Écrit l'en-tête d'abord si le fichier n'existe pas.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`] si le registre n'a pas pu être écrit. Une dépense qu'on
    /// n'arrive pas à enregistrer est un échec : le budget d'un run se lit
    /// ici, et une ligne perdue le fait mentir.
    pub fn append(&self, row: &Row) -> Outcome<()> {
        let fresh = !self.path.exists();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| self.wrote_nothing(&e))?;
        }
        let mut text = String::new();
        if fresh {
            let _ = writeln!(text, "{HEADER}");
        }
        let _ = writeln!(text, "{}", row.render());
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| self.wrote_nothing(&e))?;
        file.write_all(text.as_bytes())
            .map_err(|e| self.wrote_nothing(&e))
    }

    /// Chaque ligne de données, découpée, en-tête écarté.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] si le fichier existe mais ne se lit pas : un
    /// registre illisible n'est pas un registre vide.
    pub fn rows(&self) -> Outcome<Vec<Vec<String>>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&self.path).map_err(|e| {
            Halt::Unreadable(format!(
                "registre illisible en {} ({e}) — ce que le harness a dépensé \
                 est inconnu",
                self.path.display()
            ))
        })?;
        Ok(text
            .lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.split('\t').map(ToString::to_string).collect())
            .collect())
    }

    fn wrote_nothing(&self, err: &std::io::Error) -> Halt {
        Halt::Failed(format!(
            "impossible d'écrire le registre {} ({err}) — la dépense de cette \
             stage n'est enregistrée nulle part",
            self.path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Tokens;

    fn row() -> Row {
        Row {
            when: "2026-09-30T12:00:00Z".to_string(),
            run: "run-1".to_string(),
            round: 3,
            task: "42".to_string(),
            stage: "code".to_string(),
            spend: Spend {
                cost_usd: Some(1.5),
                turns: Some(7),
                duration_ms: Some(45_000),
                tokens: Tokens {
                    input: Some(100),
                    output: Some(20),
                    cache_read: Some(5),
                    cache_write: Some(6),
                },
                session: Some("sess-42".to_string()),
            },
            ran_on: "macbook".to_string(),
            outcome: "ok".to_string(),
        }
    }

    #[test]
    fn a_row_has_exactly_the_columns_of_the_header() {
        assert_eq!(row().render().split('\t').count(), columns().len());
    }

    #[test]
    fn the_header_is_the_frozen_one() {
        // Gelé : les colonnes et leur ordre sont un format sur disque.
        assert_eq!(
            HEADER,
            "when\trun\tround\ttask\tstage\tcost_usd\tturns\tduration_ms\tin\t\
             out\tsession\tran_on\tcache_read\tcache_write\toutcome"
        );
        assert_eq!(columns().len(), 15);
    }

    #[test]
    fn values_land_in_the_column_the_header_names() {
        let rendered = row().render();
        let cells: Vec<&str> = rendered.split('\t').collect();
        let at = |name: &str| {
            cells[columns()
                .iter()
                .position(|c| *c == name)
                .expect("colonne connue")]
        };
        assert_eq!(at("when"), "2026-09-30T12:00:00Z");
        assert_eq!(at("round"), "03");
        assert_eq!(at("stage"), "code");
        assert_eq!(at("cost_usd"), "1.500000");
        assert_eq!(at("in"), "100");
        assert_eq!(at("out"), "20");
        assert_eq!(at("cache_read"), "5");
        assert_eq!(at("cache_write"), "6");
        assert_eq!(at("session"), "sess-42");
        assert_eq!(at("outcome"), "ok");
    }

    #[test]
    fn an_unobserved_number_is_left_blank_not_written_as_zero() {
        let blind = Row {
            spend: Spend::default(),
            ..row()
        };
        let rendered = blind.render();
        let cells: Vec<&str> = rendered.split('\t').collect();
        let at = |name: &str| {
            cells[columns()
                .iter()
                .position(|c| *c == name)
                .expect("colonne connue")]
        };
        // Un zéro se relirait comme une session gratuite ; le vide dit
        // « non mesuré », ce qui est le cas sous un porteur aveugle.
        assert_eq!(at("cost_usd"), "");
        assert_eq!(at("in"), "");
        assert_eq!(at("turns"), "");
    }

    #[test]
    fn a_tab_in_free_text_cannot_shift_the_whole_line() {
        let messy = Row {
            task: "42\tavec\tdes\ttabulations".to_string(),
            ..row()
        };
        assert_eq!(messy.render().split('\t').count(), columns().len());
    }

    #[test]
    fn a_newline_in_free_text_cannot_split_the_row_in_two() {
        let messy = Row {
            task: "42\nsur deux lignes".to_string(),
            ..row()
        };
        let rendered = messy.render();
        assert!(!rendered.contains('\n'));
        assert_eq!(rendered.split('\t').count(), columns().len());
    }

    #[test]
    fn writing_then_reading_back_gives_the_row_without_the_header() {
        let dir = std::env::temp_dir().join(format!("harness-ledger-{}", std::process::id()));
        let path = dir.join("costs.tsv");
        let _ = std::fs::remove_dir_all(&dir);
        let ledger = Ledger::new(&path);

        ledger.append(&row()).expect("1re ligne");
        ledger.append(&row()).expect("2e ligne");

        let back = ledger.rows().expect("relecture");
        assert_eq!(back.len(), 2, "l'en-tête ne compte pas comme une ligne");
        assert_eq!(back[0].len(), columns().len());

        // L'en-tête n'est écrit qu'une fois.
        let raw = std::fs::read_to_string(&path).expect("lecture");
        assert_eq!(raw.matches(HEADER).count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_ledger_that_does_not_exist_yet_reads_as_no_rows() {
        let path = std::env::temp_dir().join("harness-ledger-absent/nowhere.tsv");
        assert!(Ledger::new(&path).rows().expect("absent").is_empty());
    }
}
