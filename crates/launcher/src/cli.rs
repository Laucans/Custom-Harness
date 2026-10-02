//! Les arguments d'un run, et les variables d'environnement qui les doublent.
//!
//! **Déclarés ensemble.** Côté Python, `settings.py` lisait `os.environ` dans
//! des `default_factory`, et un test assérait que « toute variable que le code
//! lit apparaît dans un épilogue `--help` » — une règle vraie par vigilance.
//! Ici `#[arg(env = …)]` met les deux au même endroit : l'aide est engendrée
//! depuis les déclarations, donc elle ne peut plus être incomplète.

use clap::Parser;
use harness_core::domain::workspace::Strategy;

/// Le harness : fait tourner un workflow d'agent sous des portes de
/// vérification.
///
/// Quinze booléens, et c'est ce qu'une ligne de commande **est** : un drapeau
/// présent ou absent. Le conseil du lint — une machine à états, des énumérations
/// à deux variantes — produirait ici un indirect sans rien rendre plus sûr, et
/// `clap` engendre l'aide depuis ces champs-là.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Parser)]
#[command(
    name = "harness",
    version,
    about = "Fait tourner la boucle de développement agentique.",
    long_about = None,
)]
pub struct Cli {
    /// N'exécute rien et ne dépense rien : dit ce qui aurait tourné, et écrit
    /// les prompts qui seraient partis.
    #[arg(long)]
    pub dry_run: bool,

    /// Les seuls stages à faire tourner, séparés par des espaces. Vide : tous.
    #[arg(long, env = "STAGES", default_value = "")]
    pub stages: String,

    /// Forçage de modèle pour tout le run. Vide : chaque stage garde le sien.
    #[arg(long, env = "MODEL", default_value = "")]
    pub model: String,

    /// Forçage d'effort pour tout le run.
    #[arg(long, env = "EFFORT", default_value = "")]
    pub effort: String,

    /// Combien de tours au plus.
    #[arg(long, env = "MAX_ROUNDS", default_value_t = 3)]
    pub rounds: u32,

    /// La branche sur laquelle la boucle travaille et merge.
    #[arg(long, env = "INTEGRATION_BRANCH", default_value = "main_agent")]
    pub branch: String,

    /// Le mode de permission passé à `claude`.
    ///
    /// Les hooks `PreToolUse` du dépôt cible s'appliquent toujours : ce réglage
    /// ne les contourne pas.
    #[arg(long, env = "PERMISSION_MODE", default_value = "bypassPermissions")]
    pub permission_mode: String,

    /// Laisse tourner même si l'arbre de travail est sale.
    #[arg(long, env = "ALLOW_DIRTY")]
    pub allow_dirty: bool,

    /// Rejoue un stage que la reprise ferait sauter.
    #[arg(long)]
    pub restart: bool,

    /// Reprend la task que le point de reprise désigne. Par défaut, oui.
    #[arg(long)]
    pub no_resume: bool,

    /// Branche le stage de rollover : quand le milestone est fini, `/planner`
    /// ouvre l'item de roadmap suivant.
    ///
    /// **Pas le défaut**, et c'est un choix : enchaîner en non surveillé dépense
    /// un run opus et engage le projet sur un item de roadmap que personne n'a
    /// lu.
    #[arg(long, env = "ROLLOVER")]
    pub rollover: bool,

    /// Tout, y compris ce qu'une session dit en détail.
    #[arg(long, short, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Seuls les arrêts et les échecs.
    #[arg(long, short)]
    pub quiet: bool,

    // --- le workspace ------------------------------------------------------
    /// Travaille dans le dépôt d'où le run est lancé, sans cloner.
    #[arg(long)]
    pub no_workspace: bool,

    /// Ce qu'il advient du dossier de travail.
    #[arg(long, env = "WORKSPACE_STRATEGY", value_parser = strategy)]
    pub workspace_strategy: Option<Strategy>,

    /// L'URL à cloner. Vide : l'`origin` du dépôt d'où le run est lancé.
    #[arg(long, env = "WORKSPACE_URL", default_value = "")]
    pub workspace_url: String,

    /// Retrouve ce workspace par son nom. Il est alors **jamais supprimé**.
    #[arg(long, default_value = "")]
    pub use_workspace: String,

    /// Le dossier qui contient les workspaces.
    #[arg(long, env = "AGENTIC_WORKSPACES_DIR", default_value = "")]
    pub workspaces_dir: String,

    /// Garde un workspace jetable que le run supprimerait.
    #[arg(long, env = "KEEP_WORKSPACE")]
    pub keep_workspace: bool,

    /// Écrase le travail local d'un workspace réutilisé.
    ///
    /// Sans ce drapeau, un workspace qui porte quelque chose arrête le run en le
    /// nommant. C'est un humain qui prend cette sortie.
    #[arg(long)]
    pub force_reset: bool,
}

/// `Strategy::parse`, en refusant plutôt qu'en retombant sur un défaut.
///
/// Un `WORKSPACE_STRATEGY=permanant` qui retomberait sur `tmp` ferait
/// supprimer, une fois par run et sans rien dire, le workspace que l'humain
/// croyait garder.
fn strategy(text: &str) -> Result<Strategy, String> {
    Strategy::parse(text).ok_or_else(|| {
        format!(
            "{text:?} n'est pas une stratégie — connues : {}",
            Strategy::known()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_declarations_are_coherent() {
        // `debug_assert` de clap : noms en double, conflits impossibles, valeur
        // par défaut qui ne passe pas son propre `value_parser`.
        Cli::command().debug_assert();
    }

    #[test]
    fn every_environment_variable_the_run_reads_shows_up_in_the_help() {
        // Ce que le Python assérait dans un test, et que la déclaration rend
        // vrai ici : l'aide est engendrée depuis les `env = …`.
        let help = Cli::command().render_long_help().to_string();
        for variable in [
            "STAGES",
            "MODEL",
            "EFFORT",
            "MAX_ROUNDS",
            "INTEGRATION_BRANCH",
            "PERMISSION_MODE",
            "ALLOW_DIRTY",
            "ROLLOVER",
            "WORKSPACE_STRATEGY",
            "WORKSPACE_URL",
            "AGENTIC_WORKSPACES_DIR",
            "KEEP_WORKSPACE",
        ] {
            assert!(help.contains(variable), "{variable} absente de --help");
        }
    }

    #[test]
    fn a_misspelled_strategy_is_refused_rather_than_defaulted() {
        assert!(strategy("permanant").is_err());
        assert_eq!(strategy("tmp"), Ok(Strategy::Tmp));
    }

    #[test]
    fn the_defaults_are_the_ones_the_python_run_with() {
        let cli = Cli::try_parse_from(["harness"]).expect("les défauts");
        assert_eq!(cli.rounds, 3);
        assert_eq!(cli.branch, "main_agent");
        assert_eq!(cli.permission_mode, "bypassPermissions");
        assert!(!cli.dry_run);
        // Le rollover reste à brancher explicitement : il dépense un run opus.
        assert!(!cli.rollover);
        // Et aucune stratégie imposée : le défaut vient du domaine, qui ne
        // supprime rien.
        assert_eq!(cli.workspace_strategy, None);
    }

    #[test]
    fn verbose_and_quiet_cannot_be_asked_for_together() {
        assert!(Cli::try_parse_from(["harness", "--verbose", "--quiet"]).is_err());
    }
}
