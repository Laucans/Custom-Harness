//! Ce qui est vérifié avant que le premier stage soit payé.
//!
//! Chaque porte ici a coûté un run à écrire. Une faute de frappe dans une
//! étiquette ne coûte rien à trouver à ce moment-là, et un `/code` entier à
//! trouver une fois que `claude` a été facturé.
//!
//! **Une porte décide, elle n'appelle jamais `gh`/`git`/`which` elle-même** :
//! chacune tient le port dont elle a besoin, et c'est l'adaptateur qui parle au
//! système. Les portes d'outillage — `claude` sur le PATH et à la bonne
//! version, `gh` authentifié — vivent dans le lanceur : elles ne sont propres à
//! aucun workflow, et c'est le lanceur qui monte les ports.

use std::path::PathBuf;
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::disk::Disk;
use harness_core::adapters::shell::github::GitHub;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{Context, Verification};

use crate::common::labels;
use crate::dev_loop::board;
use crate::dev_loop::state::Loop;

/// Les sept étiquettes dans lesquelles tout le modèle est exprimé.
///
/// Une étiquette qui n'existe pas n'échoue **nulle part** : l'interroger rend
/// une liste vide, et une liste vide de tasks se lit « ce milestone est fini » —
/// la seule entrée qui fasse payer un `/planner`. Une faute de frappe vaut
/// d'être attrapée ici, gratuitement.
pub struct LabelsExist {
    /// De quoi lister les étiquettes du dépôt.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Verification<Loop> for LabelsExist {
    async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
        let known = self.gh.labels().await?;
        let missing: Vec<&str> = labels::LOOP
            .into_iter()
            .filter(|wanted| !known.iter().any(|here| here == wanted))
            .collect();
        if missing.is_empty() {
            return Ok(Verdict::Continue);
        }
        let fix = missing
            .iter()
            .map(|label| format!("gh label create {label}"))
            .collect::<Vec<_>>()
            .join("; ");
        Err(Halt::Halted(format!(
            "the repository is missing the label(s) {} — create them with: {fix}",
            missing.join(" ")
        )))
    }
}

/// Il y a un milestone ouvert, et GitHub a répondu quand on a demandé.
///
/// `board::read` échoue sur les deux comptes, avec la phrase qui nomme le
/// geste ; le faire tourner ici est ce qui empêche ces deux pannes d'atterrir
/// au milieu d'un round, après qu'une stage a été facturée.
pub struct MilestoneIsReachable {
    /// De quoi lire le tableau.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Verification<Loop> for MilestoneIsReachable {
    async fn verify(&self, ctx: &Context<Loop>) -> Outcome<Verdict> {
        let here = board::read(self.gh.as_ref()).await?;
        ctx.traces.debug(&format!(
            "milestone {}: {} — {} open task(s)",
            here.milestone.reference(),
            here.milestone.title,
            here.open_agents().len()
        ));
        Ok(Verdict::Continue)
    }
}

/// La table nomme des skills : leurs fichiers doivent exister.
///
/// Les `lead` comptent aussi — `/tech-analyst` n'est le nom d'aucune entrée de
/// table, donc rien d'autre ne le verrait manquer.
pub struct SkillsExist {
    /// De quoi demander si un fichier est là.
    pub disk: Rc<dyn Disk>,
    /// `.claude/skills` du workspace.
    pub skills: PathBuf,
    /// Les skills que ce run nomme — stages et `lead` confondus.
    pub named: Vec<String>,
}

#[async_trait(?Send)]
impl Verification<Loop> for SkillsExist {
    async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
        for skill in &self.named {
            let path = self.skills.join(skill).join("SKILL.md");
            if !self.disk.exists(&path) {
                return Err(Halt::Halted(format!(
                    "this run names /{skill} but .claude/skills/{skill}/SKILL.md \
                     does not exist"
                )));
            }
        }
        Ok(Verdict::Continue)
    }
}

/// Ce qu'un stage doit trouver dans le workspace pour que ses commandes de
/// vérification puissent seulement démarrer.
///
/// **La porte qui rend le passage au workspace tenable.** Un clone ne porte que
/// ce que git suit : ni les dépendances npm, ni un venv. Un workspace frais
/// passe donc toutes les autres portes — il est propre par construction — puis
/// brûle une session payante dont chaque commande de vérification répond
/// « command not found ».
///
/// Une porte coûte un `stat`. Le round qu'elle évite coûte un `/code`.
///
/// **En dernier** dans la liste : c'est la seule qui parle de ce qu'il y a
/// *dans* le workspace plutôt que de ce qu'il est, et elle n'a de sens qu'une
/// fois qu'on sait que le workspace est celui qu'on croit.
pub struct DependenciesAreInstalled {
    /// De quoi demander si un chemin est là.
    pub disk: Rc<dyn Disk>,
    /// La racine du code.
    pub root: PathBuf,
    /// Ce qui doit être là, et comment le remettre.
    pub needed: Vec<(String, String)>,
}

#[async_trait(?Send)]
impl Verification<Loop> for DependenciesAreInstalled {
    async fn verify(&self, _ctx: &Context<Loop>) -> Outcome<Verdict> {
        let missing: Vec<&(String, String)> = self
            .needed
            .iter()
            .filter(|(path, _)| !self.disk.exists(&self.root.join(path)))
            .collect();
        if missing.is_empty() {
            return Ok(Verdict::Continue);
        }
        Err(Halt::Halted(format!(
            "the workspace has no {} — nothing a clone carries. Install them in \
             {}: {}",
            missing
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            self.root.display(),
            missing
                .iter()
                .map(|(_, fix)| fix.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::FakeGitHub;
    use harness_core::domain::Issue;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;
    use std::collections::HashSet;
    use std::path::Path;

    fn ctx() -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            Loop::default(),
            Logbook::null(),
        )
    }

    /// Un disque où seuls ces chemins existent.
    struct Only(HashSet<PathBuf>);

    impl Only {
        fn with(paths: &[&str]) -> Rc<Self> {
            Rc::new(Self(paths.iter().map(PathBuf::from).collect()))
        }
    }

    impl Disk for Only {
        fn exists(&self, path: &Path) -> bool {
            self.0.contains(path)
        }

        fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
            Ok(())
        }

        fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
            Ok(())
        }

        fn dir_names(&self, _path: &Path) -> Vec<String> {
            Vec::new()
        }

        fn read_to_string(&self, _path: &Path) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn every_label_the_model_needs_is_demanded_and_the_fix_is_named() {
        let gh = Rc::new(FakeGitHub {
            labels: vec![labels::MILESTONE.to_string(), labels::AGENT.to_string()],
            ..FakeGitHub::default()
        });
        let err = LabelsExist { gh }
            .verify(&ctx())
            .await
            .expect_err("doit s'arrêter");
        assert!(err.reason().contains(labels::READY));
        assert!(
            err.reason().contains("gh label create"),
            "une porte nomme le geste"
        );
        // Ce qui est déjà là n'est pas redemandé.
        assert!(!err.reason().contains(&format!("create {}", labels::AGENT)));
    }

    #[tokio::test]
    async fn all_seven_labels_present_lets_the_run_start() {
        let gh = Rc::new(FakeGitHub {
            labels: labels::LOOP.iter().map(ToString::to_string).collect(),
            ..FakeGitHub::default()
        });
        assert_eq!(
            LabelsExist { gh }.verify(&ctx()).await.expect("un verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn an_unreadable_label_list_is_not_an_absent_label() {
        // L'écart vaut un `/planner` : « je ne sais pas » n'est pas « absente ».
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("jeton expiré".to_string())),
            ..FakeGitHub::default()
        });
        let err = LabelsExist { gh }
            .verify(&ctx())
            .await
            .expect_err("doit échouer");
        assert!(matches!(err, Halt::Unreadable(_)));
    }

    #[tokio::test]
    async fn a_reachable_milestone_passes_without_paying_anything() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![Issue {
                number: 4,
                state: "open".to_string(),
                labels: vec![labels::MILESTONE.to_string()],
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        assert_eq!(
            MilestoneIsReachable { gh }
                .verify(&ctx())
                .await
                .expect("un verdict"),
            Verdict::Continue
        );
    }

    #[tokio::test]
    async fn a_skill_the_table_names_but_the_repo_lacks_stops_the_run() {
        let gate = SkillsExist {
            disk: Only::with(&["/w/.claude/skills/code/SKILL.md"]),
            skills: PathBuf::from("/w/.claude/skills"),
            named: vec!["code".to_string(), "tech-analyst".to_string()],
        };
        let err = gate.verify(&ctx()).await.expect_err("doit s'arrêter");
        // `/tech-analyst` est un `lead` : le nom d'aucune entrée de table, donc
        // rien d'autre ne le verrait manquer.
        assert!(err.reason().contains("tech-analyst"));
    }

    #[tokio::test]
    async fn a_fresh_clone_without_dependencies_stops_before_burning_a_session() {
        // Un workspace frais passe toutes les autres portes — il est propre par
        // construction — puis brûle un /code dont chaque commande répond
        // « command not found ».
        let gate = DependenciesAreInstalled {
            disk: Only::with(&[]),
            root: PathBuf::from("/w"),
            needed: vec![("node_modules".to_string(), "npm install".to_string())],
        };
        let err = gate.verify(&ctx()).await.expect_err("doit s'arrêter");
        assert!(err.reason().contains("node_modules"));
        assert!(err.reason().contains("npm install"));
        assert!(err.reason().contains("nothing a clone carries"));
    }

    #[tokio::test]
    async fn an_installed_workspace_passes() {
        let gate = DependenciesAreInstalled {
            disk: Only::with(&["/w/node_modules"]),
            root: PathBuf::from("/w"),
            needed: vec![("node_modules".to_string(), "npm install".to_string())],
        };
        assert_eq!(
            gate.verify(&ctx()).await.expect("un verdict"),
            Verdict::Continue
        );
    }
}
