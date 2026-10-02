//! Le binaire `gh`, emballé.
//!
//! Chaque appel **nomme le dépôt** par son répertoire de travail, pour la même
//! raison que `git.rs` : `gh` résout une PR depuis son `cwd`, donc un appel
//! lancé depuis un autre checkout y résoudrait une issue homonyme.
//!
//! **Les issues passent par `gh api`**, pas par `gh issue`. Les sous-issues
//! (`issues/{n}/sub_issues`) et les dépendances
//! (`issues/{n}/dependencies/blocked_by`) n'ont aucun drapeau natif dans `gh` :
//! puisque la moitié du modèle doit de toute façon passer par l'API brute,
//! tout y passe, plutôt que de laisser un lecteur devenir quelle moitié fait
//! quoi.
//!
//! Rien ici ne décide. Lire une issue, ses commentaires, ses bloqueurs, poser
//! une étiquette : ce qu'une étiquette **signifie** est la définition d'un
//! workflow et vit chez lui.
//!
//! # L'invariant qui coûte de l'argent
//!
//! **Une lecture qui n'aboutit pas ne rend jamais une liste vide.** `[]` se lit
//! « ce milestone n'a plus aucune task ouverte », qui est exactement l'entrée
//! qui déclenche un `/planner` : un jeton expiré coûterait un run opus. Toute
//! lecture ratée rend donc [`Halt::Unreadable`], qui dit ce qu'on ne sait pas
//! et nomme le geste qui débloque.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::Value;

use crate::adapters::shell::process;
use crate::domain::{Halt, Issue, Outcome, Pr};

/// Le binaire appelé.
const BINARY: &str = "gh";

/// Assez pour un dépôt d'une personne.
///
/// Et une raison de ne pas dépendre de `--paginate` : sa sortie multi-pages
/// n'est pas un seul document JSON dans toutes les versions de `gh`, et un
/// parseur qui s'y tromperait rendrait un tableau vide — c'est-à-dire « plus
/// rien à faire ».
const PER_PAGE: u32 = 100;

/// Le plafond de ce qui se pagine.
///
/// Au-delà, la lecture échoue plutôt que de rendre une liste tronquée :
/// tronquée, elle se lit comme une liste complète.
const MAX_PAGES: u32 = 10;

/// La lecture qui n'a pas abouti, dite plutôt que rendue vide.
fn unreadable(what: &str, detail: &str) -> Halt {
    Halt::Unreadable(format!(
        "lecture impossible : {what} ({detail}) — ce qu'il reste à faire est \
         inconnu, et lire ça comme « plus rien à faire » est ce qui fait ouvrir \
         au harness un item de roadmap que personne n'a demandé. Vérifier \
         `gh auth status` et le dépôt, puis relancer."
    ))
}

/// Une issue de l'API, dans la forme que le domaine sait lire.
///
/// Les étiquettes arrivent en **objets** sur la plupart des points d'entrée et
/// en **chaînes** sur certains : les deux sont acceptées, parce qu'un refus ici
/// arrêterait un run pour une différence de forme sans importance.
fn issue_from(payload: &Value) -> Outcome<Issue> {
    let number = payload
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| unreadable("une issue", "aucun champ `number` utilisable"))?;
    let labels = payload
        .get("labels")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|label| match label {
                    Value::String(name) => Some(name.clone()),
                    other => other
                        .get("name")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Issue {
        number,
        title: text_at(payload, "title"),
        state: {
            let state = text_at(payload, "state");
            if state.is_empty() {
                "open".to_string()
            } else {
                state
            }
        },
        labels,
        body: text_at(payload, "body"),
        blocked_by: Vec::new(),
    })
}

/// Un champ texte, ou le vide. `null` et absent se lisent pareil.
fn text_at(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Les métadonnées qu'une règle de saut de la revue demande, en un appel.
const PR_FIELDS: &str = "number,baseRefName,headRefName,title,url,state,isDraft";

/// Une PR de l'API, dans la forme que le domaine sait lire.
fn pr_from(payload: &Value) -> Pr {
    Pr {
        num: payload
            .get("number")
            .and_then(Value::as_u64)
            .map_or_else(String::new, |n| n.to_string()),
        base: text_at(payload, "baseRefName"),
        head: text_at(payload, "headRefName"),
        title: text_at(payload, "title"),
        url: text_at(payload, "url"),
        state: {
            let state = text_at(payload, "state");
            if state.is_empty() {
                "OPEN".to_string()
            } else {
                state
            }
        },
        draft: payload
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// Les issues d'une réponse de liste, **PR exclues**.
///
/// `/issues` rend aussi les pull requests — GitHub les modélise comme des
/// issues. Une PR portant par mégarde l'étiquette demandée entrerait alors dans
/// la réponse, et un appelant qui cherche son milestone la prendrait pour un ;
/// la clé `pull_request` est ce qui les distingue.
fn issues_from(payload: &Value, what: &str) -> Outcome<Vec<Issue>> {
    rows_of(payload, what)?
        .iter()
        .filter(|row| row.get("pull_request").is_none())
        .map(issue_from)
        .collect()
}

/// Les PR **mergées** de la réponse, dans l'ordre reçu.
///
/// Le filtre s'arrête là : `merged_at` est une propriété de l'API, et la lire
/// est le travail de cet adaptateur. Quelle PR *vaut preuve de livraison* est
/// la convention d'un workflow, et se décide chez lui.
fn merged_from(payload: &Value, what: &str) -> Outcome<Vec<Issue>> {
    rows_of(payload, what)?
        .iter()
        .filter(|row| !matches!(row.get("merged_at"), None | Some(Value::Null)))
        .map(issue_from)
        .collect()
}

/// Le tableau d'une réponse. `null` compte comme vide, autre chose est illisible.
fn rows_of<'a>(payload: &'a Value, what: &str) -> Outcome<&'a [Value]> {
    match payload {
        Value::Array(rows) => Ok(rows),
        Value::Null => Ok(&[]),
        other => Err(unreadable(
            what,
            &format!("attendu un tableau, reçu {other}"),
        )),
    }
}

/// Ce que le harness demande à `gh`, et rien de plus.
#[async_trait(?Send)]
pub trait GitHub {
    /// `gh` est-il authentifié ?
    ///
    /// # Errors
    /// Si `gh` n'a pas pu être lancé.
    async fn authenticated(&self) -> Outcome<bool>;

    /// `owner/name`, demandé une fois puis retenu.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si `gh` ne sait pas dire de quel dépôt il s'agit.
    async fn repo(&self) -> Outcome<String>;

    /// Les étiquettes que le dépôt porte, par leur nom.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si la lecture n'aboutit pas.
    async fn labels(&self) -> Outcome<Vec<String>>;

    /// Cette issue.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si la lecture n'aboutit pas.
    async fn issue(&self, number: u64) -> Outcome<Issue>;

    /// Les issues portant cette étiquette, PR exclues.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si la lecture n'aboutit pas.
    async fn issues_labelled(&self, label: &str, state: &str) -> Outcome<Vec<Issue>>;

    /// Les sous-issues de celle-ci.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si la lecture n'aboutit pas.
    async fn sub_issues(&self, number: u64) -> Outcome<Vec<Issue>>;

    /// Ce qui bloque cette issue, avec l'état de chaque bloqueur.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si la lecture n'aboutit pas.
    async fn blocked_by(&self, number: u64) -> Outcome<Vec<Issue>>;

    /// Les mêmes tasks, chacune portant ses bloqueurs.
    ///
    /// Une requête par task : le point d'entrée qui liste les sous-issues ne
    /// dit rien des dépendances, et décider sans elles reviendrait à lire
    /// « rien ne bloque ».
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si une lecture n'aboutit pas.
    async fn with_blockers(&self, tasks: Vec<Issue>) -> Outcome<Vec<Issue>>;

    /// Les PR mergées sur `base`, les plus récemment touchées d'abord.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si la lecture n'aboutit pas — « l'API est en
    /// panne » et « rien n'a été livré » mènent à des décisions opposées.
    async fn merged_prs(&self, base: &str) -> Outcome<Vec<Issue>>;

    /// Le corps de chaque commentaire, du plus ancien au plus récent.
    ///
    /// # Errors
    /// [`Halt::Unreadable`] si la lecture n'aboutit pas, ou s'il y a plus de
    /// commentaires que la pagination n'en couvre.
    async fn issue_comments(&self, number: u64) -> Outcome<Vec<String>>;

    /// Pose une étiquette.
    ///
    /// # Errors
    /// [`Halt::Halted`] si GitHub refuse.
    async fn add_label(&self, number: u64, label: &str) -> Outcome<()>;

    /// Retire une étiquette.
    ///
    /// # Errors
    /// [`Halt::Halted`] si GitHub refuse.
    async fn remove_label(&self, number: u64, label: &str) -> Outcome<()>;

    /// Réécrit le corps d'une issue — pour une task, son SPEC.
    ///
    /// # Errors
    /// [`Halt::Halted`] si GitHub refuse.
    async fn set_body(&self, number: u64, body: &str) -> Outcome<()>;

    /// Poste un commentaire sur une issue.
    ///
    /// # Errors
    /// [`Halt::Halted`] si GitHub refuse.
    async fn post_issue_comment(&self, number: u64, body: &str) -> Outcome<()>;

    /// Ferme une issue.
    ///
    /// # Errors
    /// [`Halt::Halted`] si GitHub refuse.
    async fn close_issue(&self, number: u64) -> Outcome<()>;

    // --- ce que la revue de PR demande, et elle seule ----------------------
    //
    // Par `gh pr view`/`gh pr comment`, pas par `gh api` : une revue n'a pas
    // besoin des sous-issues ni des dépendances, et ces deux sous-commandes
    // rendent déjà la forme qu'il faut. Leurs échecs sont [`Halt::Failed`], pas
    // [`Halt::Unreadable`] : il n'y a ici aucune liste vide qui pourrait se
    // relire comme « plus rien à faire » — une PR qu'on ne peut pas lire est
    // un échec ordinaire, pas une ambiguïté.

    /// Les métadonnées d'une PR, par son numéro ou son URL.
    ///
    /// # Errors
    /// [`Halt::Failed`] si `gh` ne peut pas la lire.
    async fn pr(&self, pr_ref: &str) -> Outcome<Pr>;

    /// Le corps de tous les commentaires d'une PR, **concaténés tels quels**.
    ///
    /// Une chaîne et non une liste, à dessein : la seule chose qui en est
    /// faite est une recherche de sous-chaîne (le marqueur d'une revue déjà
    /// postée), et c'est exactement ce que rend `gh pr view --json comments -q
    /// .comments[].body`.
    ///
    /// # Errors
    /// [`Halt::Failed`] si `gh` ne peut pas les lire.
    async fn pr_comments(&self, num: &str) -> Outcome<String>;

    /// Poste un commentaire sur une PR, depuis un fichier.
    ///
    /// Un fichier et non une chaîne : le texte est déjà gardé sur disque avant
    /// cet appel, pour qu'il survive à un `gh` qui échoue.
    ///
    /// # Errors
    /// [`Halt::Failed`] si GitHub refuse.
    async fn post_pr_comment(&self, num: &str, body_file: &Path) -> Outcome<()>;
}

/// `gh`, appelé depuis un dépôt donné.
pub struct GhCli {
    root: PathBuf,
    /// `owner/name`, retenu après la première demande.
    ///
    /// Un `OnceLock` et non un `RefCell` : la sémantique est exactement
    /// celle-ci — écrit une fois, relu ensuite — et il est `Sync`, donc il ne
    /// rend pas les futurs de ce module non-`Send` pour une mémoïsation. Il
    /// supprime aussi le piège du `RefCell` : il n'y a pas d'emprunt à garder
    /// ouvert à travers un `await`.
    name: OnceLock<String>,
}

impl GhCli {
    /// `gh`, sur ce dépôt.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            name: OnceLock::new(),
        }
    }

    async fn gh(&self, args: &[String]) -> Outcome<process::Ran> {
        process::run(BINARY, args, &self.root).await
    }

    /// Une lecture d'API. `path` est relatif au dépôt.
    ///
    /// Le dépôt est résolu ici plutôt que par l'appelant : c'est la seule façon
    /// qu'un nom de dépôt illisible soit un échec propagé comme les autres.
    async fn read(&self, what: &str, path: &str, query: &[(&str, String)]) -> Outcome<Value> {
        let repo = self.repo().await?;
        let mut args = vec![
            "api".to_string(),
            format!("repos/{repo}/{path}"),
            "-X".to_string(),
            "GET".to_string(),
        ];
        for (key, value) in query {
            args.push("-f".to_string());
            args.push(format!("{key}={value}"));
        }
        let ran = self.gh(&args).await?;
        if !ran.ok() {
            return Err(unreadable(what, &ran.why()));
        }
        let body = ran.out();
        if body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(body).map_err(|e| unreadable(what, &e.to_string()))
    }

    /// Une écriture d'API.
    ///
    /// Rien n'est réessayé et rien n'est défait : un changement à moitié
    /// appliqué est plus facile à finir à la main qu'à deviner.
    async fn write(
        &self,
        what: &str,
        method: &str,
        path: &str,
        fields: &[(&str, String)],
    ) -> Outcome<()> {
        let repo = self.repo().await?;
        let url = format!("repos/{repo}/{path}");
        let mut args = vec![
            "api".to_string(),
            "-X".to_string(),
            method.to_string(),
            url.clone(),
        ];
        for (key, value) in fields {
            args.push("-f".to_string());
            args.push(format!("{key}={value}"));
        }
        let ran = self.gh(&args).await?;
        if ran.ok() {
            return Ok(());
        }
        Err(Halt::Halted(format!(
            "GitHub a refusé de {what} ({method} {url}) : {}. Rien n'est \
             réessayé et rien n'est défait ici — un changement à moitié \
             appliqué est plus facile à finir à la main qu'à deviner.",
            ran.why()
        )))
    }

    /// Les issues d'un point d'entrée qui en liste, lues telles quelles.
    async fn issue_list(&self, what: &str, path: &str) -> Outcome<Vec<Issue>> {
        let payload = self
            .read(what, path, &[("per_page", PER_PAGE.to_string())])
            .await?;
        rows_of(&payload, what)?.iter().map(issue_from).collect()
    }
}

#[async_trait(?Send)]
impl GitHub for GhCli {
    async fn authenticated(&self) -> Outcome<bool> {
        Ok(self
            .gh(&["auth".to_string(), "status".to_string()])
            .await?
            .ok())
    }

    async fn repo(&self) -> Outcome<String> {
        if let Some(name) = self.name.get() {
            return Ok(name.clone());
        }
        let ran = self
            .gh(&[
                "repo".to_string(),
                "view".to_string(),
                "--json".to_string(),
                "nameWithOwner".to_string(),
                "-q".to_string(),
                ".nameWithOwner".to_string(),
            ])
            .await?;
        if !ran.ok() || ran.out().is_empty() {
            return Err(unreadable("le nom du dépôt", &ran.why()));
        }
        let name = ran.out().to_string();
        // `set` échoue si un autre appel a gagné la course : la valeur est la
        // même, donc il n'y a rien à rattraper.
        let _ = self.name.set(name.clone());
        Ok(name)
    }

    async fn labels(&self) -> Outcome<Vec<String>> {
        let what = "les étiquettes du dépôt";
        let payload = self
            .read(what, "labels", &[("per_page", PER_PAGE.to_string())])
            .await?;
        Ok(rows_of(&payload, what)?
            .iter()
            .filter_map(|row| row.get("name").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect())
    }

    async fn issue(&self, number: u64) -> Outcome<Issue> {
        let what = format!("l'issue #{number}");
        let payload = self.read(&what, &format!("issues/{number}"), &[]).await?;
        issue_from(&payload)
    }

    async fn issues_labelled(&self, label: &str, state: &str) -> Outcome<Vec<Issue>> {
        let what = format!("les issues {label}");
        let payload = self
            .read(
                &what,
                "issues",
                &[
                    ("labels", label.to_string()),
                    ("state", state.to_string()),
                    ("per_page", PER_PAGE.to_string()),
                ],
            )
            .await?;
        issues_from(&payload, &what)
    }

    async fn sub_issues(&self, number: u64) -> Outcome<Vec<Issue>> {
        self.issue_list(
            &format!("les sous-issues de #{number}"),
            &format!("issues/{number}/sub_issues"),
        )
        .await
    }

    async fn blocked_by(&self, number: u64) -> Outcome<Vec<Issue>> {
        self.issue_list(
            &format!("ce qui bloque #{number}"),
            &format!("issues/{number}/dependencies/blocked_by"),
        )
        .await
    }

    async fn with_blockers(&self, tasks: Vec<Issue>) -> Outcome<Vec<Issue>> {
        let mut out = Vec::with_capacity(tasks.len());
        for mut task in tasks {
            task.blocked_by = self.blocked_by(task.number).await?;
            out.push(task);
        }
        Ok(out)
    }

    async fn merged_prs(&self, base: &str) -> Outcome<Vec<Issue>> {
        let what = format!("les pull requests mergées sur {base}");
        let payload = self
            .read(
                &what,
                "pulls",
                &[
                    ("state", "closed".to_string()),
                    ("base", base.to_string()),
                    ("sort", "updated".to_string()),
                    ("direction", "desc".to_string()),
                    ("per_page", PER_PAGE.to_string()),
                ],
            )
            .await?;
        merged_from(&payload, &what)
    }

    async fn issue_comments(&self, number: u64) -> Outcome<Vec<String>> {
        let what = format!("les commentaires de l'issue #{number}");
        let mut bodies = Vec::new();
        // Paginé, à la différence des autres lectures : GitHub rend les
        // commentaires du plus ancien au plus récent, donc une première page
        // seule perd les derniers — et un compteur de round lu sur une liste
        // tronquée repart en arrière, par-dessus du travail déjà payé.
        for page in 1..=MAX_PAGES {
            let payload = self
                .read(
                    &what,
                    &format!("issues/{number}/comments"),
                    &[
                        ("per_page", PER_PAGE.to_string()),
                        ("page", page.to_string()),
                    ],
                )
                .await?;
            let rows = rows_of(&payload, &what)?;
            let count = rows.len();
            bodies.extend(rows.iter().map(|row| text_at(row, "body")));
            if count < PER_PAGE as usize {
                return Ok(bodies);
            }
        }
        Err(unreadable(
            &what,
            &format!("plus de {} commentaires", MAX_PAGES * PER_PAGE),
        ))
    }

    async fn add_label(&self, number: u64, label: &str) -> Outcome<()> {
        self.write(
            &format!("étiqueter #{number} avec {label}"),
            "POST",
            &format!("issues/{number}/labels"),
            &[("labels[]", label.to_string())],
        )
        .await
    }

    async fn remove_label(&self, number: u64, label: &str) -> Outcome<()> {
        self.write(
            &format!("retirer {label} de #{number}"),
            "DELETE",
            &format!("issues/{number}/labels/{label}"),
            &[],
        )
        .await
    }

    async fn set_body(&self, number: u64, body: &str) -> Outcome<()> {
        self.write(
            &format!("réécrire le corps de #{number}"),
            "PATCH",
            &format!("issues/{number}"),
            &[("body", body.to_string())],
        )
        .await
    }

    async fn post_issue_comment(&self, number: u64, body: &str) -> Outcome<()> {
        self.write(
            &format!("commenter #{number}"),
            "POST",
            &format!("issues/{number}/comments"),
            &[("body", body.to_string())],
        )
        .await
    }

    async fn close_issue(&self, number: u64) -> Outcome<()> {
        self.write(
            &format!("fermer #{number}"),
            "PATCH",
            &format!("issues/{number}"),
            &[("state", "closed".to_string())],
        )
        .await
    }

    async fn pr(&self, pr_ref: &str) -> Outcome<Pr> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "view".to_string(),
                pr_ref.to_string(),
                "--json".to_string(),
                PR_FIELDS.to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot read PR {pr_ref} — {}",
                ran.why()
            )));
        }
        serde_json::from_str::<Value>(ran.out())
            .map_err(|e| Halt::Failed(format!("cannot read PR {pr_ref} — {e}")))
            .map(|value| pr_from(&value))
    }

    async fn pr_comments(&self, num: &str) -> Outcome<String> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "view".to_string(),
                num.to_string(),
                "--json".to_string(),
                "comments".to_string(),
                "-q".to_string(),
                ".comments[].body".to_string(),
            ])
            .await?;
        if !ran.ok() {
            return Err(Halt::Failed(format!(
                "cannot tell whether PR #{num} was already reviewed — {}",
                ran.why()
            )));
        }
        Ok(ran.stdout)
    }

    async fn post_pr_comment(&self, num: &str, body_file: &Path) -> Outcome<()> {
        let ran = self
            .gh(&[
                "pr".to_string(),
                "comment".to_string(),
                num.to_string(),
                "--body-file".to_string(),
                body_file.display().to_string(),
            ])
            .await?;
        if ran.ok() {
            return Ok(());
        }
        Err(Halt::Failed(format!(
            "exit {}: {}",
            ran.code.unwrap_or(-1),
            ran.why()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- les étiquettes, en objets ou en chaînes ----------------------------

    #[test]
    fn labels_are_read_whether_they_arrive_as_objects_or_strings() {
        let as_objects = json!({
            "number": 12,
            "labels": [{ "name": "pipeline:agent" }, { "name": "pipeline:ready" }]
        });
        let as_strings = json!({
            "number": 12,
            "labels": ["pipeline:agent", "pipeline:ready"]
        });
        let wanted = vec!["pipeline:agent".to_string(), "pipeline:ready".to_string()];
        assert_eq!(issue_from(&as_objects).expect("objets").labels, wanted);
        assert_eq!(issue_from(&as_strings).expect("chaînes").labels, wanted);
    }

    #[test]
    fn a_missing_number_is_unreadable_rather_than_a_zero() {
        let err = issue_from(&json!({ "title": "sans numéro" })).expect_err("doit échouer");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn a_null_body_reads_as_empty_not_as_the_word_null() {
        let issue = issue_from(&json!({ "number": 1, "body": null })).expect("parse");
        assert!(issue.body.is_empty());
    }

    #[test]
    fn a_missing_state_defaults_to_open_so_work_is_not_lost() {
        let issue = issue_from(&json!({ "number": 1 })).expect("parse");
        assert!(issue.is_open());
    }

    // --- l'invariant : jamais une liste vide sur une panne -----------------

    #[test]
    fn a_null_list_is_genuinely_empty() {
        // `gh` rend `null` quand il n'y a rien : inoffensif, et différent
        // d'une panne.
        assert!(
            issues_from(&Value::Null, "les issues")
                .expect("null")
                .is_empty()
        );
    }

    #[test]
    fn a_response_that_is_not_a_list_is_unreadable_not_empty() {
        // Le mode de panne que ça évite : une réponse inattendue lue comme
        // « plus rien à faire », qui fait payer un /planner.
        let err = issues_from(&json!({ "message": "Bad credentials" }), "les issues")
            .expect_err("doit échouer");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[test]
    fn the_unreadable_message_names_the_gesture_that_unblocks() {
        let Halt::Unreadable(said) = unreadable("les issues", "401") else {
            panic!("doit être Unreadable");
        };
        assert!(said.contains("gh auth status"));
    }

    // --- les PR ------------------------------------------------------------

    #[test]
    fn pull_requests_are_filtered_out_of_an_issue_listing() {
        // GitHub modélise les PR comme des issues : une PR portant par
        // mégarde l'étiquette demandée serait prise pour un milestone.
        let payload = json!([
            { "number": 1, "title": "une vraie issue" },
            { "number": 2, "title": "une PR", "pull_request": { "url": "..." } },
        ]);
        let issues = issues_from(&payload, "les issues").expect("parse");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 1);
    }

    #[test]
    fn only_merged_pull_requests_come_back() {
        let payload = json!([
            { "number": 1, "merged_at": "2026-09-30T10:00:00Z" },
            { "number": 2, "merged_at": null },
            { "number": 3 },
        ]);
        let merged = merged_from(&payload, "les PR").expect("parse");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].number, 1);
    }

    #[test]
    fn a_pr_is_read_from_its_own_fields_not_the_issue_shape() {
        let payload = json!({
            "number": 32,
            "baseRefName": "main_agent",
            "headRefName": "feat/cities",
            "title": "feat(db): table cities",
            "url": "https://github.com/o/r/pull/32",
            "state": "MERGED",
            "isDraft": false
        });
        let pr = pr_from(&payload);
        assert_eq!(pr.num, "32");
        assert_eq!(pr.base, "main_agent");
        assert_eq!(pr.state, "MERGED");
        assert!(!pr.draft);
    }

    #[test]
    fn a_missing_state_on_a_pr_defaults_to_open() {
        let pr = pr_from(&json!({ "number": 1 }));
        assert_eq!(pr.state, "OPEN");
    }

    #[test]
    fn an_unreadable_pull_request_list_does_not_read_as_nothing_delivered() {
        // « l'API est en panne » et « rien n'a été livré » mènent à des
        // décisions opposées.
        let err =
            merged_from(&json!({ "message": "rate limited" }), "les PR").expect_err("doit échouer");
        assert!(matches!(err, Halt::Unreadable(_)));
    }
}
