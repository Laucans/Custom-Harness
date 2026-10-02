//! Le câblage de la boucle : ce qui branche les vrais adaptateurs.
//!
//! **Le seul endroit du paquet qui construit quelque chose de concret.** Un
//! `GhCli`, un `GitCli`, une fabrique de sessions `claude`, un registre sur le
//! disque — rien de tout cela n'est nommé ailleurs, et c'est ce qui permet au
//! reste de n'avoir qu'une couture de test par port.
//!
//! L'ordre y est celui que le montage impose : les portes d'outillage tournent
//! **avant** le montage parce qu'un clone de plusieurs centaines de mégaoctets
//! ne doit pas précéder la découverte que `gh` n'est pas authentifié ; celles
//! qui parlent du workspace tournent **après**, parce qu'elles parlent de ce
//! qu'il y a dedans.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use harness_core::adapters::agent::SessionFactory;
use harness_core::adapters::agent::claude_cli::ClaudeCliFactory;
use harness_core::adapters::agent::rehearsal::Rehearsal;
use harness_core::adapters::shell::disk::{Disk, RealDisk};
use harness_core::adapters::shell::git::{GitCli, GitRepos, Repo, Repos};
use harness_core::adapters::shell::github::{GhCli, GitHub};
use harness_core::adapters::store::checkpoint::Checkpoint;
use harness_core::adapters::store::spending::Spending;
use harness_core::domain::Verdict;
use harness_core::domain::workspace::{Wanted, Workspace};
use harness_core::execution::provisioning::{Mount, Provisioner, Run};
use harness_core::execution::{Context, Gate, Guarded, Settings, Verification};
use harness_core::traces::{Logbook, Sink, Verbosity};
use harness_workflows::dev_loop::actions::{MarkWaitingMerge, PickTask};
use harness_workflows::dev_loop::state::Loop;
use harness_workflows::dev_loop::wiring::Wiring;
use harness_workflows::dev_loop::workflow::DevLoop;
use harness_workflows::dev_loop::{gates, preflight, round::TaskRound, stages};

use crate::cli::Cli;
use crate::sink::Both;
use crate::spending::{self, LedgerSpending};
use crate::tooling;

/// Ce que ce dépôt doit avoir d'installé pour qu'un stage puisse se vérifier.
///
/// Rien de tout cela n'est suivi par git — c'est précisément pourquoi un clone
/// frais ne l'a pas, et pourquoi la porte existe.
fn installed() -> Vec<(String, String)> {
    vec![("node_modules".to_string(), "npm install".to_string())]
}

/// Ce qu'un run a fini par faire.
pub struct Ran {
    /// Le verdict du workflow.
    pub verdict: Verdict,
    /// Où le journal a été écrit.
    pub log: PathBuf,
}

/// Fait tourner la boucle : préflight, montage, tours, démontage.
///
/// # Errors
///
/// Le premier [`harness_core::domain::Halt`] qui arrête le run — une porte qui
/// refuse, une session qui s'arrête d'elle-même, un quota épuisé. Son
/// `exit_code` est ce que l'appelant rend au système.
pub async fn run(cli: &Cli, here: &Path) -> harness_core::domain::Outcome<Ran> {
    let run_id = spending::run_id();
    let source = Workspace::new(here);
    let sink = Rc::new(
        Both::new(&source.loop_dir().join(&run_id).join("run.log")).map_err(|e| {
            harness_core::domain::Halt::Failed(format!("le journal ne s'ouvre pas : {e}"))
        })?,
    );
    let log_path = sink.path().to_path_buf();
    let log = Logbook::new(Rc::clone(&sink) as Rc<dyn Sink>, verbosity(cli));

    let disk: Rc<dyn Disk> = Rc::new(RealDisk);
    let repos: Rc<dyn Repos> = Rc::new(GitRepos);
    // Les portes d'outillage d'abord, sur le dépôt d'où le run est lancé : un
    // clone ne doit pas précéder la découverte que `gh` n'est pas authentifié.
    let outside: Rc<dyn GitHub> = Rc::new(GhCli::new(source.root()));
    tooling(cli, &source, &disk, outside)
        .verify(&bare(cli, log.clone()))
        .await?;

    let mount = mount(cli, &source, &repos, &disk, &run_id, &log).await?;
    let workspace = &mount.workspace;
    let provisioner = Provisioner {
        repos: Rc::clone(&repos),
        disk: Rc::clone(&disk),
    };

    let outcome = turns(cli, workspace, &run_id, Rc::clone(&disk), &log).await;
    // Le démontage passe quoi qu'il arrive : c'est lui qui garde un workspace
    // porteur de travail, et qui supprime celui qui n'en porte pas.
    provisioner.unmount(&mount, &log).await;
    Ok(Ran {
        verdict: outcome?,
        log: log_path,
    })
}

/// Le niveau demandé pour la console. Le fichier garde tout.
const fn verbosity(cli: &Cli) -> Verbosity {
    if cli.verbose {
        Verbosity::Verbose
    } else if cli.quiet {
        Verbosity::Quiet
    } else {
        Verbosity::Normal
    }
}

/// Un contexte sans état utile, pour les portes qui n'en lisent pas.
fn bare(cli: &Cli, log: Logbook) -> Context<Loop> {
    Context::new(settings(cli), Loop::default(), log)
}

fn settings(cli: &Cli) -> Settings {
    Settings {
        dry_run: cli.dry_run,
        stages: cli.stages.clone(),
    }
}

/// Les portes d'outillage, dans l'ordre où elles se vérifient.
///
/// Elles se supposent : demander à `gh` quelles étiquettes existent n'a pas de
/// sens tant qu'on ne sait pas s'il est authentifié, et la version de `claude`
/// passe en tête parce qu'elle est la seule qui ne coûte aucun appel réseau.
fn tooling(cli: &Cli, source: &Workspace, disk: &Rc<dyn Disk>, gh: Rc<dyn GitHub>) -> Gate<Loop> {
    let git: Rc<dyn Repo> = Rc::new(GitCli::new(source.root()));
    Gate {
        name: "outillage",
        checks: vec![
            Box::new(tooling::ClaudeIsRecentEnough::default()),
            Box::new(tooling::GhIsAuthenticated { gh }),
            Box::new(tooling::TheIntegrationBranch {
                git: Rc::clone(&git),
                branch: cli.branch.clone(),
                // Dans un clone, la branche courante est posée par le montage ;
                // l'exiger du dépôt de l'humain l'obligerait à basculer pour un
                // run qui ne touche pas à son arbre.
                check_current: cli.no_workspace,
            }),
            Box::new(tooling::CiTriggersOnTheBranch {
                disk: Rc::clone(disk),
                path: source.root().join(".github/workflows/ci.yml"),
                branch: cli.branch.clone(),
            }),
            Box::new(tooling::WorkingTreeIsClean {
                git,
                // Un run qui travaille dans un clone ne touche pas à l'arbre de
                // l'humain : la porte n'a de sens que sur place.
                allowed: cli.allow_dirty || !cli.no_workspace,
            }),
        ],
    }
}

/// Le workspace de ce run, monté — ou le dépôt d'ici si on n'en veut pas.
async fn mount(
    cli: &Cli,
    source: &Workspace,
    repos: &Rc<dyn Repos>,
    disk: &Rc<dyn Disk>,
    run_id: &str,
    log: &Logbook,
) -> harness_core::domain::Outcome<Mount> {
    if cli.no_workspace {
        log.say("workspace: aucun — le run travaille dans ce dépôt");
        return Ok(Mount::in_place(source.clone()));
    }
    let wanted = Wanted {
        strategy: cli.strategy().map_err(harness_core::domain::Halt::Halted)?,
        url: cli.workspace_url.clone(),
        id: cli.use_workspace.clone(),
        base: cli.workspaces_dir.clone(),
        keep: cli.keep_workspace,
        force_reset: cli.force_reset,
    };
    Provisioner {
        repos: Rc::clone(repos),
        disk: Rc::clone(disk),
    }
    .mount(
        &Run {
            source,
            wanted: &wanted,
            branch: &cli.branch,
            run_id,
            dry_run: cli.dry_run,
        },
        log,
    )
    .await
}

/// Le workflow, câblé contre le workspace monté, puis exécuté.
async fn turns(
    cli: &Cli,
    workspace: &Workspace,
    run_id: &str,
    disk: Rc<dyn Disk>,
    log: &Logbook,
) -> harness_core::domain::Outcome<Verdict> {
    // Tout ce qui suit parle au **clone**, pas au dépôt d'où le run est lancé :
    // c'est là que les sessions éditent, et c'est là que `gh` doit répondre.
    let gh: Rc<dyn GitHub> = Rc::new(GhCli::new(workspace.root()));
    let sessions: Rc<dyn SessionFactory> = if cli.dry_run {
        // Un dry-run est un choix de câblage, pas une branche du framework.
        Rc::new(Rehearsal::new(log.clone()))
    } else {
        Rc::new(ClaudeCliFactory::new(
            workspace.root(),
            &cli.permission_mode,
        ))
    };
    let spending: Rc<dyn Spending> = Rc::new(LedgerSpending::new(
        &workspace.ledger(),
        run_id,
        &spending::hostname().await,
    ));
    let store = Rc::new(Checkpoint::new(&workspace.loop_dir()));
    // Lu avant le premier tour : le pointeur dit quelle task un run interrompu
    // était en train de faire, et elle **prime** sur le choix du tableau.
    let resuming = if cli.no_resume {
        None
    } else {
        store.pointer()?.task
    };
    if let Some(task) = &resuming {
        log.say(&format!(
            "reprise : task #{task} (--no-resume pour l'ignorer)"
        ));
    }

    let wiring = Wiring {
        gh: Rc::clone(&gh),
        sessions,
        spending,
        integration_branch: cli.branch.clone(),
        model: cli.model.clone(),
        effort: cli.effort.clone(),
        restart: cli.restart,
    };
    announce(cli, &wiring, run_id, log);

    let built = DevLoop {
        pre: workspace_gates(cli, workspace, &wiring, Rc::clone(&disk), Rc::clone(&gh)),
        budget: cli.rounds,
        rounds: rounds(cli, &wiring, resuming),
        store: Some(Rc::clone(&store)),
        flow_id: run_id.to_string(),
    };
    let mut ctx = Context::new(settings(cli), Loop::default(), log.clone());
    built.execute(&mut ctx).await
}

/// Les portes qui parlent du workspace monté, et de ce qu'il y a dedans.
fn workspace_gates(
    cli: &Cli,
    workspace: &Workspace,
    wiring: &Wiring,
    disk: Rc<dyn Disk>,
    gh: Rc<dyn GitHub>,
) -> Gate<Loop> {
    Gate {
        name: "préflight de la boucle",
        checks: vec![
            Box::new(preflight::LabelsExist { gh: Rc::clone(&gh) }),
            Box::new(preflight::MilestoneIsReachable { gh }),
            Box::new(preflight::SkillsExist {
                disk: Rc::clone(&disk),
                skills: workspace.root().join(".claude/skills"),
                named: named_skills(wiring),
            }),
            // En dernier : la seule qui parle de ce qu'il y a *dans* le
            // workspace plutôt que de ce qu'il est.
            Box::new(preflight::DependenciesAreInstalled {
                disk,
                root: workspace.root().to_path_buf(),
                needed: if cli.dry_run { Vec::new() } else { installed() },
            }),
        ],
    }
}

/// Les skills que ce run nomme : les stages, et les commandes qui les ouvrent.
///
/// Un `lead` n'est le nom d'aucune entrée de table — `/tech-analyst` ouvre le
/// stage `code` —, donc rien d'autre ne le verrait manquer.
fn named_skills(wiring: &Wiring) -> Vec<String> {
    let mut named: Vec<String> = stages::table(wiring, 1)
        .iter()
        .map(|stage| stage.name.clone())
        .collect();
    named.push("tech-analyst".to_string());
    named
}

/// La fabrique de rounds que le workflow appelle une fois par tour.
///
/// La reprise ne vaut que pour le **premier** : les tours suivants choisissent
/// dans le tableau, et redonner la clé de reprise ferait rejouer la même task.
fn rounds(cli: &Cli, wiring: &Wiring, resuming: Option<String>) -> Box<dyn Fn(u32) -> TaskRound> {
    let gh = Rc::clone(&wiring.gh);
    let sessions = Rc::clone(&wiring.sessions);
    let spending = Rc::clone(&wiring.spending);
    let branch = wiring.integration_branch.clone();
    let model = wiring.model.clone();
    let effort = wiring.effort.clone();
    let restart = wiring.restart;
    let rollover = cli.rollover;
    let stages_filter = cli.stages.clone();
    Box::new(move |turn| {
        let wiring = Wiring {
            gh: Rc::clone(&gh),
            sessions: Rc::clone(&sessions),
            spending: Rc::clone(&spending),
            integration_branch: branch.clone(),
            model: model.clone(),
            effort: effort.clone(),
            restart,
        };
        TaskRound {
            turn,
            pick: PickTask {
                gh: Rc::clone(&gh),
                resuming: if turn == 1 { resuming.clone() } else { None },
            },
            stages: stages::table(&wiring, turn),
            rollover: rollover.then(|| stages::planner(&wiring, turn)),
            delivered: MarkWaitingMerge {
                gh: Rc::clone(&gh),
                integration_branch: branch.clone(),
            },
            post: Gate {
                name: "le round doit avoir livré",
                checks: vec![Box::new(gates::AMergedPrClosesTheTask {
                    gh: Rc::clone(&gh),
                    integration_branch: branch.clone(),
                    code_runs: code_runs(&stages_filter),
                    stages: stages_filter.clone(),
                })],
            },
        }
    })
}

/// `--stages` laisse-t-il tourner le seul stage qui livre une task ?
///
/// Ce que la post-condition du round a besoin de savoir pour que « rien ne
/// marque la livraison » ne se lise pas comme un bug alors que c'est le filtre
/// qui a retiré `code`.
fn code_runs(stages: &str) -> bool {
    stages.is_empty() || stages.split_whitespace().any(|name| name == "code")
}

/// Dit ce que ce run va faire, une fois toutes les portes passées.
fn announce(cli: &Cli, wiring: &Wiring, run_id: &str, log: &Logbook) {
    let table = stages::table(wiring, 1);
    let cfg = settings(cli);
    log.say(&format!(
        "run {run_id} — branche {}, {} round(s){}",
        cli.branch,
        cli.rounds,
        if cli.dry_run { " [dry-run]" } else { "" }
    ));
    log.say(&format!("pipeline: {}", stages::summary(&table, &cfg)));
    // Un run avec `--stages code` ne disait rien des stages qu'il laissait
    // tomber, ce qui se relit comme un pipeline qui les a perdus.
    let left_out = stages::filtered_out(&table, &cfg);
    if !left_out.is_empty() {
        log.say(&format!(
            "laissés de côté par --stages : {}",
            left_out.join(" ")
        ));
    }
    log.say(&format!(
        "mode de permission : {} (les hooks PreToolUse du dépôt cible \
         s'appliquent toujours)",
        cli.permission_mode
    ));
    if cli.rollover {
        log.say("rollover branché : un milestone fini fera tourner /planner");
    }
}

#[cfg(test)]
mod tests {
    //! Ce qui se teste ici est le **câblage** : quel round reçoit quoi, quel
    //! stage est branché, quelle porte reçoit quel réglage. Les règles elles-
    //! mêmes sont testées là où elles vivent, contre les faux de leur crate.
    //!
    //! Les trois ports **refusent** plutôt que de ne rien faire : monter un
    //! round ne doit ouvrir aucune session, ne rien consigner, ne rien demander
    //! à GitHub, et un port muet laisserait passer le contraire.

    use super::*;
    use async_trait::async_trait;
    use harness_core::adapters::agent::{Session, SessionSpec};
    use harness_core::adapters::store::spending::Entry;
    use harness_core::domain::{Halt, Issue, Outcome};
    use serial_test::serial;

    struct NoGitHub;

    #[async_trait(?Send)]
    impl GitHub for NoGitHub {
        async fn authenticated(&self) -> Outcome<bool> {
            Err(refused())
        }
        async fn repo(&self) -> Outcome<String> {
            Err(refused())
        }
        async fn labels(&self) -> Outcome<Vec<String>> {
            Err(refused())
        }
        async fn issue(&self, _number: u64) -> Outcome<Issue> {
            Err(refused())
        }
        async fn issues_labelled(&self, _label: &str, _state: &str) -> Outcome<Vec<Issue>> {
            Err(refused())
        }
        async fn sub_issues(&self, _number: u64) -> Outcome<Vec<Issue>> {
            Err(refused())
        }
        async fn blocked_by(&self, _number: u64) -> Outcome<Vec<Issue>> {
            Err(refused())
        }
        async fn with_blockers(&self, _tasks: Vec<Issue>) -> Outcome<Vec<Issue>> {
            Err(refused())
        }
        async fn merged_prs(&self, _base: &str) -> Outcome<Vec<Issue>> {
            Err(refused())
        }
        async fn issue_comments(&self, _number: u64) -> Outcome<Vec<String>> {
            Err(refused())
        }
        async fn add_label(&self, _number: u64, _label: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn remove_label(&self, _number: u64, _label: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn set_body(&self, _number: u64, _body: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn post_issue_comment(&self, _number: u64, _body: &str) -> Outcome<()> {
            Err(refused())
        }
        async fn close_issue(&self, _number: u64) -> Outcome<()> {
            Err(refused())
        }
    }

    struct NoSessions;

    #[async_trait(?Send)]
    impl SessionFactory for NoSessions {
        async fn open(&self, _spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
            Err(refused())
        }
    }

    struct Nowhere;

    impl Spending for Nowhere {
        fn record(&self, _entry: &Entry<'_>) -> Outcome<()> {
            Err(refused())
        }
    }

    fn refused() -> Halt {
        Halt::Failed("aucun appel ne doit partir dans ce test".to_string())
    }

    fn wiring() -> Wiring {
        Wiring {
            gh: Rc::new(NoGitHub),
            sessions: Rc::new(NoSessions),
            spending: Rc::new(Nowhere),
            integration_branch: "main_agent".to_string(),
            model: String::new(),
            effort: String::new(),
            restart: false,
        }
    }

    fn cli(args: &[&str]) -> Cli {
        let mut full = vec!["harness"];
        full.extend_from_slice(args);
        <Cli as clap::Parser>::try_parse_from(full).expect("des arguments valides")
    }

    #[test]
    fn the_run_names_the_lead_skills_the_table_does_not() {
        // `/tech-analyst` ouvre le stage `code` : ce n'est le nom d'aucune
        // entrée de table, donc rien d'autre ne le verrait manquer.
        let named = named_skills(&wiring());
        assert!(named.contains(&"tech-analyst".to_string()));
        assert!(named.contains(&"code".to_string()));
        assert!(named.contains(&"create-test".to_string()));
    }

    #[test]
    #[serial]
    fn the_resume_key_is_only_given_to_the_first_turn() {
        // La redonner ferait rejouer la même task à chaque tour du budget.
        let built = rounds(&cli(&[]), &wiring(), Some("11".to_string()));
        assert_eq!(built(1).pick.resuming.as_deref(), Some("11"));
        assert_eq!(built(2).pick.resuming, None);
    }

    #[test]
    #[serial]
    fn each_turn_gets_its_own_number_for_the_ledger_and_the_journal() {
        let built = rounds(&cli(&[]), &wiring(), None);
        assert_eq!(built(3).turn, 3);
    }

    #[test]
    #[serial]
    fn the_rollover_stage_is_only_wired_when_asked_for() {
        let off = rounds(&cli(&[]), &wiring(), None);
        assert!(off(1).rollover.is_none(), "un run opus n'est pas un défaut");
        let on = rounds(&cli(&["--rollover"]), &wiring(), None);
        assert!(on(1).rollover.is_some());
    }

    #[test]
    #[serial]
    fn the_table_stays_whole_and_the_filter_lives_in_the_gates() {
        // `--stages` ne retire pas d'entrée : il fait sauter un stage *en le
        // disant*. Retirer l'entrée rendrait le journal illisible — un pipeline
        // qui aurait perdu deux stages sans explication.
        let built = rounds(&cli(&["--stages", "code"]), &wiring(), None);
        assert_eq!(built(1).stages.len(), 3);
    }

    #[test]
    fn leaving_code_out_of_stages_is_what_the_delivery_gate_must_know() {
        // Sans ça, « rien ne marque la livraison » se lit comme un bug alors
        // que c'est `--stages` qui a retiré le seul stage qui livre.
        assert!(code_runs(""), "vide veut dire tout");
        assert!(code_runs("business-analyst code"));
        assert!(!code_runs("business-analyst"));
    }

    #[test]
    #[serial]
    fn a_dry_run_asks_for_no_installed_dependencies() {
        // Rien n'est exécuté, donc aucune commande de vérification ne tournera,
        // donc exiger `node_modules` refuserait un dry-run parfaitement utile.
        assert!(cli(&["--dry-run"]).dry_run);
        assert!(!cli(&[]).dry_run);
        assert!(!installed().is_empty());
    }

    #[test]
    #[serial]
    fn the_console_level_follows_the_flags_and_the_file_keeps_everything() {
        assert_eq!(verbosity(&cli(&[])), Verbosity::Normal);
        assert_eq!(verbosity(&cli(&["--verbose"])), Verbosity::Verbose);
        assert_eq!(verbosity(&cli(&["--quiet"])), Verbosity::Quiet);
    }

    #[test]
    #[serial]
    fn a_clone_does_not_demand_a_clean_tree_in_the_humans_checkout() {
        // Le run ne touche pas à son arbre : l'exiger propre l'obligerait à
        // committer pour un run qui travaille ailleurs.
        let here = Workspace::new(Path::new("/depot"));
        let disk: Rc<dyn Disk> = Rc::new(RealDisk);
        // On ne peut pas lire l'intérieur d'une porte ; ce qu'on fige est le
        // réglage qui la rend permissive, et il dépend de `--no-workspace`.
        let _ = tooling(&cli(&[]), &here, &disk, Rc::new(NoGitHub));
        assert!(!cli(&[]).no_workspace);
        assert!(cli(&["--no-workspace"]).no_workspace);
    }
}
