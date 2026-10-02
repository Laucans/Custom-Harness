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
    #[arg(long, env = "ALLOW_DIRTY", num_args = 0..=1, default_missing_value = "true", default_value = "false", value_parser = truthy)]
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
    #[arg(long, env = "ROLLOVER", num_args = 0..=1, default_missing_value = "true", default_value = "false", value_parser = truthy)]
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

    /// Ce qu'il advient du dossier de travail. Vide : le défaut du domaine
    /// ([`Strategy::Permanent`]), lu par [`Cli::strategy`].
    ///
    /// En `String`, pas en `Option<Strategy>` avec un `value_parser` qui
    /// rejette — même défaut que les trois booléens juste au-dessus, et même
    /// cause : `clap` appelle le parseur dès que la variable existe, même
    /// vide, et `Strategy::parse("")` refuse à bon droit une orthographe
    /// inconnue.
    #[arg(long, env = "WORKSPACE_STRATEGY", default_value = "")]
    pub workspace_strategy: String,

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
    #[arg(long, env = "KEEP_WORKSPACE", num_args = 0..=1, default_missing_value = "true", default_value = "false", value_parser = truthy)]
    pub keep_workspace: bool,

    /// Écrase le travail local d'un workspace réutilisé.
    ///
    /// Sans ce drapeau, un workspace qui porte quelque chose arrête le run en le
    /// nommant. C'est un humain qui prend cette sortie.
    #[arg(long)]
    pub force_reset: bool,
}

impl Cli {
    /// La stratégie de workspace demandée, ou le défaut du domaine si rien
    /// n'est donné.
    ///
    /// # Errors
    ///
    /// Une erreur nommant la faute si `workspace_strategy` n'est ni vide ni
    /// une stratégie connue — refusée plutôt que lue comme le défaut, pour
    /// la même raison que [`Strategy::parse`] : `WORKSPACE_STRATEGY=permanant`
    /// qui retomberait sur `tmp` ferait supprimer, une fois et sans rien dire,
    /// le workspace que l'humain croyait garder.
    pub fn strategy(&self) -> Result<Strategy, String> {
        if self.workspace_strategy.is_empty() {
            return Ok(Strategy::Permanent);
        }
        strategy(&self.workspace_strategy)
    }
}

/// Un booléen d'environnement, aux orthographes usuelles.
///
/// `clap` exige la chaîne littérale `true`/`false` pour tout champ `bool`
/// combiné à `env` — ni `1`, ni vide, ni `action = SetTrue` n'y changent rien,
/// vérifié en isolation avant ce correctif. Les trois drapeaux qui s'écrivent
/// aussi en variable d'environnement (`ALLOW_DIRTY`, `ROLLOVER`,
/// `KEEP_WORKSPACE`) passent donc par ce parseur plutôt que par le type
/// `bool` nu de `clap` : `num_args = 0..=1` et `default_missing_value =
/// "true"` gardent `--rollover` valable seul en ligne de commande, et ce
/// parseur accepte en plus ce qu'une variable d'environnement écrit en
/// pratique.
///
/// Le vide vaut faux **volontairement** : c'est ce qu'une ligne `ROLLOVER=`
/// non remplie dans `.env.local` écrit, et c'est le cas le plus courant —
/// pas une faute de frappe. Une vraie faute (`ROLLOVER=flase`) reste refusée,
/// pour la même raison que [`strategy`] refuse plutôt que de retomber sur un
/// défaut : une variable mal orthographiée ne doit pas se lire comme un
/// silence.
fn truthy(text: &str) -> Result<bool, String> {
    match text.trim().to_lowercase().as_str() {
        "" | "0" | "false" | "no" => Ok(false),
        "1" | "true" | "yes" => Ok(true),
        other => Err(format!(
            "{other:?} n'est pas une valeur booléenne — connues : 1/true/yes, \
             0/false/no, ou vide"
        )),
    }
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
    use serial_test::serial;

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
    #[serial]
    fn the_defaults_are_the_ones_the_python_run_with() {
        let cli = Cli::try_parse_from(["harness"]).expect("les défauts");
        assert_eq!(cli.rounds, 3);
        assert_eq!(cli.branch, "main_agent");
        assert_eq!(cli.permission_mode, "bypassPermissions");
        assert!(!cli.dry_run);
        // Le rollover reste à brancher explicitement : il dépense un run opus.
        assert!(!cli.rollover);
        // Et aucune stratégie imposée en ligne de commande : c'est
        // `Cli::strategy` qui retombe sur le défaut du domaine, qui ne
        // supprime rien.
        assert_eq!(cli.workspace_strategy, "");
        assert_eq!(cli.strategy(), Ok(Strategy::Permanent));
    }

    #[test]
    #[serial]
    fn an_empty_workspace_strategy_defaults_rather_than_erroring() {
        // Le même défaut que les trois booléens : `clap` appellerait
        // `Strategy::parse("")` dès que `WORKSPACE_STRATEGY=` existe, même
        // vide, si le champ restait un `Option<Strategy>` à `value_parser`.
        let cli = Cli::try_parse_from_with_env(["harness"], &[("WORKSPACE_STRATEGY", "")])
            .expect("ne doit pas échouer à l'analyse des arguments");
        assert_eq!(cli.strategy(), Ok(Strategy::Permanent));
    }

    #[test]
    #[serial]
    fn a_misspelled_workspace_strategy_is_refused_at_use_not_defaulted() {
        let cli = Cli::try_parse_from_with_env(["harness"], &[("WORKSPACE_STRATEGY", "permanant")])
            .expect("l'analyse des arguments ne rejette plus — Cli::strategy le fait");
        assert!(cli.strategy().is_err());
    }

    #[test]
    #[serial]
    fn verbose_and_quiet_cannot_be_asked_for_together() {
        assert!(Cli::try_parse_from(["harness", "--verbose", "--quiet"]).is_err());
    }

    #[test]
    #[serial]
    fn a_bool_flag_still_takes_no_value_on_the_command_line() {
        let cli = Cli::try_parse_from(["harness", "--rollover"]).expect("un drapeau nu");
        assert!(cli.rollover);
    }

    #[test]
    fn truthy_accepts_the_usual_spellings() {
        for yes in ["1", "true", "yes", "TRUE", " yes "] {
            assert_eq!(truthy(yes), Ok(true), "{yes:?}");
        }
        for no in ["0", "false", "no", "FALSE"] {
            assert_eq!(truthy(no), Ok(false), "{no:?}");
        }
    }

    #[test]
    fn truthy_reads_empty_as_false_because_that_is_what_an_unfilled_env_var_is() {
        // Ce que `.env.local` écrit pour une variable non remplie : une ligne
        // `ROLLOVER=` vide. `clap` exige `true`/`false` littéral pour tout
        // `bool` combiné à `env` — ni `1`, ni vide, ni `action = SetTrue` n'y
        // changent rien, ce qui faisait échouer `--allow-dirty` seul dès que
        // `.env.local` existait, même avec les trois variables vides. C'est
        // ce que `truthy` contourne.
        assert_eq!(truthy(""), Ok(false));
    }

    #[test]
    fn truthy_refuses_a_typo_rather_than_reading_it_as_unset() {
        // Même logique que `strategy` : une variable mal orthographiée ne
        // doit pas se lire comme un silence.
        assert!(truthy("flase").is_err());
    }

    #[test]
    #[serial]
    fn an_empty_or_truthy_environment_variable_does_not_break_the_flag() {
        for (value, expect_allow_dirty) in [("", false), ("1", true), ("true", true)] {
            let cli = Cli::try_parse_from_with_env(["harness"], &[("ALLOW_DIRTY", value)])
                .expect("ne doit pas échouer");
            assert_eq!(cli.allow_dirty, expect_allow_dirty, "ALLOW_DIRTY={value:?}");
        }
    }

    /// `Cli::try_parse_from`, des variables d'environnement posées pour la
    /// durée de l'appel.
    ///
    /// `std::env::set_var` touche un état global du process, partagé entre les
    /// threads que `cargo test` utilise pour les autres tests de ce fichier —
    /// les poser et les retirer dans le même appel, plutôt que dans un test
    /// qui continuerait après, est ce qui empêche une fuite vers un test
    /// voisin même en cas de panique entre les deux.
    trait TryParseWithEnv: Sized {
        fn try_parse_from_with_env<I, T>(
            args: I,
            vars: &[(&str, &str)],
        ) -> Result<Self, clap::Error>
        where
            I: IntoIterator<Item = T>,
            T: Into<std::ffi::OsString> + Clone;
    }

    impl TryParseWithEnv for Cli {
        #[allow(unsafe_code)]
        fn try_parse_from_with_env<I, T>(
            args: I,
            vars: &[(&str, &str)],
        ) -> Result<Self, clap::Error>
        where
            I: IntoIterator<Item = T>,
            T: Into<std::ffi::OsString> + Clone,
        {
            // SAFETY: posées puis retirées avant de rendre la main, dans le
            // même appel — aucun autre test ne peut observer l'état
            // intermédiaire.
            unsafe {
                for (var, value) in vars {
                    std::env::set_var(var, value);
                }
            }
            let parsed = Self::try_parse_from(args);
            unsafe {
                for (var, _) in vars {
                    std::env::remove_var(var);
                }
            }
            parsed
        }
    }

    /// Chaque variable que `.env.example` déclare, telle qu'elle y est écrite.
    ///
    /// Une seule source pour les deux tests qui suivent, pour que le jour où
    /// une ligne s'ajoute à `.env.example` sans qu'on pense à l'ajouter ici,
    /// l'écart soit visible — pas pour relire le fichier automatiquement :
    /// `.env.example` est un gabarit pour un humain, ce tableau est ce que le
    /// code promet d'accepter.
    const ENV_EXAMPLE: &[(&str, &str)] = &[
        ("STAGES", ""),
        ("MODEL", ""),
        ("EFFORT", ""),
        ("MAX_ROUNDS", "3"),
        ("INTEGRATION_BRANCH", "main_agent"),
        ("PERMISSION_MODE", "bypassPermissions"),
        ("ALLOW_DIRTY", ""),
        ("ROLLOVER", ""),
        ("WORKSPACE_STRATEGY", ""),
        ("WORKSPACE_URL", ""),
        ("AGENTIC_WORKSPACES_DIR", ""),
        ("KEEP_WORKSPACE", ""),
    ];

    #[test]
    #[serial]
    fn a_dot_env_local_freshly_copied_from_the_template_parses_without_error() {
        // Le scénario qui a cassé deux fois de suite en usage réel avant que ce
        // test existe : `.env.local` copié tel quel depuis `.env.example`, la
        // plupart des lignes laissées vides. Les deux bugs — `bool` + `env`, et
        // `Option<Strategy>` + `env` — se seraient tous les deux montrés ici.
        let cli = Cli::try_parse_from_with_env(["harness"], ENV_EXAMPLE)
            .expect("un .env.local frais doit toujours parser");
        assert_eq!(cli.strategy(), Ok(Strategy::Permanent));
    }

    #[test]
    fn every_line_of_the_template_is_covered_by_the_regression_test() {
        // Garde contre l'écart inverse : une ligne ajoutée à `.env.example`
        // sans être ajoutée à `ENV_EXAMPLE` ci-dessus ne serait plus couverte,
        // en silence.
        let documented = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.example"),
        )
        .expect(".env.example doit se lire");
        for line in documented.lines() {
            let Some((name, _)) = line.split_once('=') else {
                continue;
            };
            if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                assert!(
                    ENV_EXAMPLE.iter().any(|(known, _)| *known == name),
                    "{name} est dans .env.example mais pas dans ENV_EXAMPLE"
                );
            }
        }
    }
}
