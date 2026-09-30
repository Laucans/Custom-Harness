//! Où en est le harness, et comment il reprend.
//!
//! Deux magasins, et c'est délibéré :
//!
//! - un **pointeur** de deux lignes (`task=`, `flow_id=`), qui reste lisible au
//!   `cat` et effaçable à la main. Il existe parce qu'un état que personne ne
//!   peut lire est un état que personne ne débogue ;
//! - les **états**, un fichier JSONL par flow, une ligne par étape.
//!
//! **Ce qui n'est pas porté : sqlite.** Le schéma du Python
//! (`flow_states(flow_uuid, method_name, timestamp, state_json)`) n'existait
//! que par héritage du `@persist` d'un moteur de graphe, et ce moteur est mort
//! avant la migration. Du JSONL garde la propriété qui comptait — une ligne par
//! étape, donc une reprise ratée reste lisible après coup — sans traîner une
//! dépendance C pour un fichier qu'on ouvre deux fois par round.
//!
//! Synchrone, à la différence de `Session` et `Repo` : ceux-là lancent des
//! processus, ce qui est lent et n'existe qu'en async chez tokio. Écrire deux
//! kilo-octets sur un disque local n'a rien à gagner d'un `await`, et le
//! prétendre asynchrone donnerait une fausse idée de ce que ça coûte.
//!
//! # L'invariant qui coûte de l'argent
//!
//! **Un magasin illisible n'est jamais « rien n'a tourné ».** Les deux réponses
//! valent une session de `/code` d'écart : la seconde fait repayer une stage
//! qui a peut-être déjà mergé. Un magasin absent, un flow inconnu et un
//! identifiant vide veulent dire la même chose, inoffensive, et rendent `None` ;
//! tout le reste rend [`Halt::Unreadable`].

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::domain::{Halt, Outcome};

/// Le point de reprise, tel que le pointeur le dit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pointer {
    /// La task en cours, si le harness en avait une.
    pub task: Option<String>,
    /// Le flow dont les états portent le détail.
    pub flow_id: Option<String>,
}

/// Le pointeur, tel qu'il s'écrit : deux lignes, lisibles au `cat`.
#[must_use]
fn render_pointer(task: &str, flow_id: &str) -> String {
    format!("task={task}\nflow_id={flow_id}\n")
}

/// Le pointeur, tel qu'il se relit.
///
/// Une ligne qu'on ne comprend pas est ignorée : le fichier est fait pour être
/// édité à la main, et une faute de frappe ne doit pas arrêter un run.
#[must_use]
fn parse_pointer(text: &str) -> Pointer {
    let mut found = Pointer::default();
    for line in text.lines() {
        if let Some(task) = line.strip_prefix("task=") {
            found.task = Some(task.trim().to_string()).filter(|t| !t.is_empty());
        } else if let Some(flow) = line.strip_prefix("flow_id=") {
            found.flow_id = Some(flow.trim().to_string()).filter(|f| !f.is_empty());
        }
    }
    found
}

/// Le magasin de reprise, dans un répertoire.
pub struct Checkpoint {
    dir: PathBuf,
}

impl Checkpoint {
    /// Le magasin dans ce répertoire. Rien n'est créé avant la première
    /// écriture.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn pointer_path(&self) -> PathBuf {
        self.dir.join("state")
    }

    /// Un fichier par flow. L'identifiant est assaini : il vient d'un
    /// workflow, et un `../` dedans écrirait ailleurs que dans le magasin.
    fn flow_path(&self, flow_id: &str) -> PathBuf {
        let safe: String = flow_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("flow-{safe}.jsonl"))
    }

    /// Le point de reprise, ou un pointeur vide s'il n'y en a pas.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] si le fichier existe mais ne se lit pas.
    pub fn pointer(&self) -> Outcome<Pointer> {
        let path = self.pointer_path();
        if !path.exists() {
            return Ok(Pointer::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| unreadable(&path, &e.to_string()))?;
        Ok(parse_pointer(&text))
    }

    /// Écrit le point de reprise.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`] si le pointeur n'a pas pu être écrit.
    pub fn set_pointer(&self, task: &str, flow_id: &str) -> Outcome<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| wrote_nothing(&self.dir, &e.to_string()))?;
        let path = self.pointer_path();
        std::fs::write(&path, render_pointer(task, flow_id))
            .map_err(|e| wrote_nothing(&path, &e.to_string()))
    }

    /// La task est livrée : le point de reprise n'a plus rien à décrire.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`] si le pointeur existe et n'a pas pu être supprimé — le
    /// laisser en place ferait reprendre une task déjà finie.
    pub fn clear(&self) -> Outcome<()> {
        let path = self.pointer_path();
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(wrote_nothing(&path, &e.to_string())),
        }
    }

    /// Ajoute l'état du round après une étape.
    ///
    /// Une ligne par étape plutôt qu'une mise à jour : [`Checkpoint::load`] lit
    /// la dernière, et garder les précédentes rend une reprise ratée lisible
    /// après coup.
    ///
    /// # Errors
    ///
    /// [`Halt::Failed`] si l'état n'a pas pu être écrit.
    pub fn save(&self, flow_id: &str, step: &str, state: &Value) -> Outcome<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| wrote_nothing(&self.dir, &e.to_string()))?;
        let path = self.flow_path(flow_id);
        let line = serde_json::json!({ "step": step, "state": state });
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| wrote_nothing(&path, &e.to_string()))?;
        writeln!(file, "{line}").map_err(|e| wrote_nothing(&path, &e.to_string()))
    }

    /// L'état le plus récent de ce flow, ou `None`.
    ///
    /// `None` veut dire « rien n'a encore tourné » — magasin absent, flow
    /// inconnu, identifiant vide. Toutes ces réponses sont inoffensives.
    ///
    /// # Errors
    ///
    /// [`Halt::Unreadable`] si le magasin existe mais ne se décode pas. **Ce
    /// n'est pas la même chose que `None`** : l'écart vaut une session de
    /// `/code`, et le confondre ferait repayer une stage déjà mergée.
    pub fn load(&self, flow_id: &str) -> Outcome<Option<Value>> {
        if flow_id.is_empty() {
            return Ok(None);
        }
        let path = self.flow_path(flow_id);
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path).map_err(|e| unreadable(&path, &e.to_string()))?;
        let Some(last) = text.lines().rfind(|line| !line.trim().is_empty()) else {
            // Un fichier vide : personne n'a encore écrit d'étape.
            return Ok(None);
        };
        let record: Value =
            serde_json::from_str(last).map_err(|e| unreadable(&path, &e.to_string()))?;
        record
            .get("state")
            .cloned()
            .map(Some)
            .ok_or_else(|| unreadable(&path, "la dernière ligne n'a pas de champ `state`"))
    }
}

/// Ce qu'on dit quand on ne sait pas où en est le harness.
fn unreadable(path: &Path, detail: &str) -> Halt {
    Halt::Unreadable(format!(
        "état de reprise illisible en {} ({detail}) — quelles stages ont déjà \
         tourné est inconnu, et lire ça comme « aucune » ferait repayer une \
         stage qui a peut-être déjà mergé. Inspecter ou supprimer le fichier, \
         ou relancer en forçant la reprise de cette task depuis le début.",
        path.display()
    ))
}

fn wrote_nothing(path: &Path, detail: &str) -> Halt {
    Halt::Failed(format!(
        "impossible d'écrire {} ({detail}) — le harness ne pourrait pas \
         reprendre là où il s'arrête",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Un répertoire à nous, effacé à la fin. Le vrai disque, pas un double :
    /// ce module *est* le système de fichiers, et un faux ne prouverait rien.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("harness-checkpoint-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Self(path)
        }

        fn store(&self) -> Checkpoint {
            Checkpoint::new(&self.0)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // --- le pointeur -------------------------------------------------------

    #[test]
    fn the_pointer_stays_two_readable_lines() {
        assert_eq!(render_pointer("42", "flow-9"), "task=42\nflow_id=flow-9\n");
    }

    #[test]
    fn a_pointer_round_trips() {
        let dir = Dir::new("pointer");
        let store = dir.store();
        store.set_pointer("42", "flow-9").expect("écriture");
        assert_eq!(
            store.pointer().expect("relecture"),
            Pointer {
                task: Some("42".to_string()),
                flow_id: Some("flow-9".to_string()),
            }
        );
    }

    #[test]
    fn no_pointer_yet_is_an_empty_pointer_not_an_error() {
        let dir = Dir::new("absent");
        assert_eq!(dir.store().pointer().expect("absent"), Pointer::default());
    }

    #[test]
    fn a_hand_edited_pointer_survives_a_stray_line() {
        // Le fichier est fait pour être édité à la main : une faute de frappe
        // ne doit pas arrêter un run.
        let found = parse_pointer("task=42\nune ligne que personne n'attend\nflow_id=f1\n");
        assert_eq!(found.task.as_deref(), Some("42"));
        assert_eq!(found.flow_id.as_deref(), Some("f1"));
    }

    #[test]
    fn an_empty_value_reads_as_absent_rather_than_as_an_empty_task() {
        assert_eq!(parse_pointer("task=\nflow_id=\n"), Pointer::default());
    }

    #[test]
    fn clearing_a_pointer_that_is_already_gone_is_not_an_error() {
        let dir = Dir::new("clear");
        let store = dir.store();
        store.clear().expect("déjà absent");
        store.set_pointer("1", "f").expect("écriture");
        store.clear().expect("suppression");
        assert_eq!(store.pointer().expect("relecture"), Pointer::default());
    }

    // --- les états ---------------------------------------------------------

    #[test]
    fn the_last_step_written_is_the_one_that_comes_back() {
        let dir = Dir::new("states");
        let store = dir.store();
        store
            .save("f1", "pick-task", &json!({ "stages_done": [] }))
            .expect("1re étape");
        store
            .save(
                "f1",
                "business-analyst",
                &json!({ "stages_done": ["business-analyst"] }),
            )
            .expect("2e étape");

        let state = store.load("f1").expect("relecture").expect("un état");
        assert_eq!(state, json!({ "stages_done": ["business-analyst"] }));
    }

    #[test]
    fn every_step_is_kept_so_a_failed_resume_stays_readable() {
        let dir = Dir::new("history");
        let store = dir.store();
        store.save("f1", "a", &json!({ "n": 1 })).expect("a");
        store.save("f1", "b", &json!({ "n": 2 })).expect("b");
        let raw = std::fs::read_to_string(dir.0.join("flow-f1.jsonl")).expect("lecture");
        assert_eq!(raw.lines().count(), 2);
    }

    #[test]
    fn nothing_has_run_yet_reads_as_none_in_all_its_harmless_forms() {
        let dir = Dir::new("nothing");
        let store = dir.store();
        // Magasin absent, flow inconnu, identifiant vide : trois façons de
        // dire la même chose inoffensive.
        assert!(store.load("f1").expect("absent").is_none());
        assert!(store.load("").expect("vide").is_none());
        store.save("f1", "a", &json!({})).expect("a");
        assert!(store.load("inconnu").expect("autre flow").is_none());
    }

    #[test]
    fn a_store_that_does_not_decode_is_unreadable_never_nothing_ran() {
        // Le mode de panne que ça évite : repayer un /code qui a déjà mergé.
        let dir = Dir::new("corrupt");
        let store = dir.store();
        std::fs::create_dir_all(&dir.0).expect("mkdir");
        std::fs::write(dir.0.join("flow-f1.jsonl"), "ceci n'est pas du json\n").expect("écriture");
        let err = store.load("f1").expect_err("doit échouer");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn a_record_without_a_state_field_is_unreadable_too() {
        let dir = Dir::new("nostate");
        let store = dir.store();
        std::fs::create_dir_all(&dir.0).expect("mkdir");
        std::fs::write(dir.0.join("flow-f1.jsonl"), "{\"step\":\"a\"}\n").expect("écriture");
        assert!(matches!(
            store.load("f1").expect_err("doit échouer"),
            Halt::Unreadable(_)
        ));
    }

    #[test]
    fn an_empty_store_file_is_nothing_ran_not_unreadable() {
        let dir = Dir::new("emptyfile");
        let store = dir.store();
        std::fs::create_dir_all(&dir.0).expect("mkdir");
        std::fs::write(dir.0.join("flow-f1.jsonl"), "").expect("écriture");
        assert!(store.load("f1").expect("vide").is_none());
    }

    #[test]
    fn a_flow_id_cannot_escape_the_store_directory() {
        // L'identifiant vient d'un workflow : un `../` dedans écrirait ailleurs.
        let dir = Dir::new("escape");
        let store = dir.store();
        let path = store.flow_path("../../ailleurs");
        assert_eq!(path.parent(), Some(dir.0.as_path()));
        assert!(!path.display().to_string().contains(".."));
    }
}
