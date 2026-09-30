//! Proposition C : un processus par action, recousu par `--resume`.
//!
//! Chaque tour est un `claude -p --output-format json`. Le premier appel crée
//! la conversation ; les suivants la reprennent par `--resume <session_id>`,
//! l'identifiant étant appris de la réponse du premier plutôt qu'imposé —
//! c'est le geste que la doc `headless` décrit, et ça évite de se demander ce
//! qu'un `--session-id` déjà pris ferait.
//!
//! « Session ouverte » est donc une fiction assumée : la continuité vit sur le
//! disque, dans le transcript, pas dans un processus vivant. C'est ce que
//! `docs/SESSION-CARRIER.md` a arbitré — le moins de code pour du coût exact
//! et des frontières de tour gratuites, avec la proposition A (tmux) comme
//! destination si l'attachabilité devient nécessaire.
//!
//! # Coût : lire `total_cost_usd` sans le sommer à l'aveugle
//!
//! **Depuis Claude Code v2.1.277**, un appel qui reprend une session rend le
//! total de **toute la conversation**, dépenses des appels précédents
//! comprises. Le coût d'une stage est donc la valeur du **dernier** tour, et
//! sommer les tours double-compterait. Avant cette version, chaque appel ne
//! rendait que le sien, et il fallait sommer.
//!
//! Cet adaptateur ne tranche pas : il rapporte fidèlement ce que le tour a
//! dit, dans [`Reply::cost`]. C'est au registre de dépenses — qui n'existe
//! pas encore — d'accumuler, et **il devra vérifier la version** plutôt que
//! de supposer. Une porte de préflight sur `claude --version` coûte un appel
//! local ; un `costs.tsv` faux ne se voit pas.
//!
//! `total_cost_usd` est par ailleurs une **estimation côté client**, calculée
//! d'une table de prix embarquée, pas une donnée de facturation. Bon pour un
//! budget, jamais pour facturer qui que ce soit.

use std::path::{Path, PathBuf};
use std::process::Output;

use async_trait::async_trait;
use serde::Deserialize;

use crate::adapters::agent::{Reply, Session, SessionFactory, SessionSpec};
use crate::domain::{Halt, Outcome, Spend, Tokens, markers};

/// Le binaire appelé. Nommé ici pour qu'un test puisse le lire.
const BINARY: &str = "claude";

/// Les bouts de phrase qui font lire un échec comme un quota épuisé.
///
/// En un seul endroit : c'est une heuristique sur du texte d'erreur, donc
/// elle dérivera, et le jour où elle dérive on veut un seul endroit à
/// corriger. Cherchée uniquement dans un tour **déjà en échec** — une session
/// qui réussit en parlant de « rate limit » n'est pas un quota épuisé.
const QUOTA_PHRASES: [&str; 4] = ["usage limit", "rate limit", "quota", "too many requests"];

/// Ce qu'on lit du JSON de `--output-format json`.
///
/// Tous les champs sont optionnels à dessein : Claude Code en ajoute au fil
/// des versions, et un champ inconnu ne doit pas faire échouer un tour qui
/// s'est bien passé.
#[derive(Debug, Deserialize)]
struct CliResult {
    #[serde(default)]
    subtype: String,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    num_turns: Option<u32>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    usage: CliUsage,
}

/// Les jetons, tels que le message `result` les rapporte.
///
/// **Sous-compte les subagents** : la doc est explicite, `usage` ne couvre que
/// la boucle principale alors que `total_cost_usd` inclut les subagents. Le
/// stage `code` en lance, donc ces jetons-là sont un plancher, pas un total.
/// C'est le coût qu'il faut lire pour un budget, pas les jetons.
// Les noms sont ceux de l'API, pas les nôtres : les renommer pour faire
// plaisir à `struct_field_names` ferait mentir le `Deserialize`.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Default, Deserialize)]
struct CliUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

/// Une conversation menée par appels successifs au binaire `claude`.
pub struct ClaudeCli {
    cwd: PathBuf,
    model: String,
    effort: String,
    permission_mode: String,
    /// Appris de la réponse du premier tour. `None` = la conversation n'existe
    /// pas encore, donc pas de `--resume` à passer.
    session_id: Option<String>,
}

impl ClaudeCli {
    /// Une conversation qui n'a pas encore eu lieu.
    #[must_use]
    pub fn new(cwd: PathBuf, spec: &SessionSpec, permission_mode: &str) -> Self {
        Self {
            cwd,
            model: spec.model.clone(),
            effort: spec.effort.clone(),
            permission_mode: permission_mode.to_string(),
            session_id: None,
        }
    }

    /// Les arguments de ce tour.
    ///
    /// Pur, et séparé de l'appel : c'est la partie qui se teste sans dépenser
    /// un centime, et c'est là que vit la seule vraie logique — passer
    /// `--resume` ou non.
    ///
    /// Le prompt voyage en argument plutôt que sur stdin. Quelques kilo-octets
    /// tiennent largement sous `ARG_MAX` ; si un préambule devenait énorme, ça
    /// serait le moment de le passer par stdin.
    fn argv(&self, prompt: &str) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            prompt.to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--model".to_string(),
            self.model.clone(),
            "--effort".to_string(),
            self.effort.clone(),
            "--permission-mode".to_string(),
            self.permission_mode.clone(),
        ];
        if let Some(id) = &self.session_id {
            args.push("--resume".to_string());
            args.push(id.clone());
        }
        args
    }
}

/// Comment classer un tour qui a échoué.
///
/// Un quota n'est ni « réparé » ni « abandonné » : c'est le même travail à
/// relancer plus tard, inchangé. Le distinguer d'un échec est ce qui évite de
/// rejouer une session qui n'avait rien de cassé.
fn halt_for(subtype: &str, text: &str) -> Halt {
    let haystack = format!("{subtype} {text}").to_lowercase();
    if QUOTA_PHRASES.iter().any(|phrase| haystack.contains(phrase)) {
        return Halt::Quota(text.to_string());
    }
    Halt::Failed(text.to_string())
}

/// Ce qu'un tour a rendu, plus l'identifiant de la conversation.
///
/// Pur : tout ce qui suit l'appel au binaire se teste en lui passant une
/// chaîne.
///
/// # Errors
///
/// - [`Halt::Failed`] si le JSON est illisible, ou si le tour a abouti sans
///   rien rendre — un tour vide n'est pas utilisable par l'action suivante ;
/// - [`Halt::Quota`] ou [`Halt::Failed`] selon ce que l'échec dit, voir
///   [`halt_for`].
fn parse(stdout: &str) -> Outcome<(Reply, String)> {
    let parsed: CliResult = serde_json::from_str(stdout.trim())
        .map_err(|e| Halt::Failed(format!("réponse illisible de {BINARY} : {e}")))?;

    if parsed.is_error {
        return Err(halt_for(&parsed.subtype, &parsed.result));
    }
    if parsed.result.trim().is_empty() {
        return Err(Halt::Failed(format!(
            "{BINARY} a abouti sans rien rendre (subtype {:?})",
            parsed.subtype
        )));
    }

    let spend = Spend {
        cost_usd: parsed.total_cost_usd,
        turns: parsed.num_turns,
        duration_ms: parsed.duration_ms,
        tokens: Tokens {
            input: parsed.usage.input_tokens,
            output: parsed.usage.output_tokens,
            cache_read: parsed.usage.cache_read_input_tokens,
            cache_write: parsed.usage.cache_creation_input_tokens,
        },
        session: (!parsed.session_id.is_empty()).then(|| parsed.session_id.clone()),
    };
    let reply = Reply {
        stop_line: markers::stop_line(&parsed.result),
        text: parsed.result,
        spend,
    };
    Ok((reply, parsed.session_id))
}

/// Ce qu'un processus qui n'a pas abouti laisse lire.
///
/// `stderr` d'abord : quand `claude` refuse un drapeau, c'est là que la raison
/// est, et le JSON de stdout est alors absent.
fn failed_process(out: &Output) -> Halt {
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let said = if stderr.is_empty() { stdout } else { stderr };
    let code = out
        .status
        .code()
        .map_or_else(|| "tué par un signal".to_string(), |c| format!("code {c}"));
    halt_for("", &format!("{BINARY} s'est arrêté ({code}) : {said}"))
}

#[async_trait(?Send)]
impl Session for ClaudeCli {
    async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
        let out = tokio::process::Command::new(BINARY)
            .args(self.argv(prompt))
            .current_dir(&self.cwd)
            .output()
            .await
            .map_err(|e| Halt::Failed(format!("{BINARY} n'a pas pu être lancé : {e}")))?;

        if !out.status.success() {
            return Err(failed_process(&out));
        }

        let (reply, session_id) = parse(&String::from_utf8_lossy(&out.stdout))?;
        // Le premier tour apprend l'identifiant ; les suivants le rendent tel
        // quel, et le réécrire ne coûte rien.
        if !session_id.is_empty() {
            self.session_id = Some(session_id);
        }
        Ok(reply)
    }
}

/// Ouvre des conversations `claude` dans un répertoire donné.
pub struct ClaudeCliFactory {
    cwd: PathBuf,
    permission_mode: String,
}

impl ClaudeCliFactory {
    /// Une fabrique qui fait travailler chaque session dans `cwd`.
    ///
    /// `cwd` est la racine du checkout du run — le clone, pas le dépôt d'où le
    /// run est lancé.
    #[must_use]
    pub fn new(cwd: &Path, permission_mode: &str) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            permission_mode: permission_mode.to_string(),
        }
    }
}

#[async_trait(?Send)]
impl SessionFactory for ClaudeCliFactory {
    /// Sous la proposition C, ouvrir ne lance rien.
    ///
    /// La conversation naît au premier `ask`. C'est la contrepartie assumée du
    /// choix : rien à démonter, rien qui fuie si une stage meurt en chemin.
    async fn open(&self, spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
        Ok(Box::new(ClaudeCli::new(
            self.cwd.clone(),
            spec,
            &self.permission_mode,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> ClaudeCli {
        ClaudeCli::new(
            PathBuf::from("/tmp/workspace"),
            &SessionSpec {
                model: "opus".to_string(),
                effort: "high".to_string(),
            },
            "bypassPermissions",
        )
    }

    // --- argv ---------------------------------------------------------------

    #[test]
    fn the_first_turn_carries_no_resume() {
        let args = cli().argv("/business-analyst");
        assert!(!args.contains(&"--resume".to_string()));
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "/business-analyst");
    }

    #[test]
    fn a_later_turn_resumes_the_session_it_learned() {
        let mut c = cli();
        c.session_id = Some("abc-123".to_string());
        let args = c.argv("/code");
        let at = args.iter().position(|a| a == "--resume").expect("--resume");
        assert_eq!(args[at + 1], "abc-123");
    }

    #[test]
    fn model_effort_and_permission_mode_all_reach_the_command_line() {
        let args = cli().argv("x").join(" ");
        assert!(args.contains("--model opus"));
        assert!(args.contains("--effort high"));
        assert!(args.contains("--permission-mode bypassPermissions"));
        assert!(args.contains("--output-format json"));
    }

    // --- parse --------------------------------------------------------------

    /// La forme documentée du JSON de `--output-format json`, champs inconnus
    /// compris : le parseur doit les ignorer, pas s'en étouffer.
    const SUCCESS: &str = r#"{
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "duration_ms": 45000,
        "duration_api_ms": 2300,
        "num_turns": 3,
        "result": "voici ce que j'ai fait\nAGENT_LOOP_OK: spec écrit",
        "session_id": "sess-42",
        "total_cost_usd": 0.1234,
        "usage": { "input_tokens": 100, "output_tokens": 20 },
        "modelUsage": { "claude-opus-5": { "costUSD": 0.1234 } },
        "un_champ_que_cette_version_ne_connait_pas": true
    }"#;

    #[test]
    fn a_successful_turn_yields_its_text_cost_and_session() {
        let (reply, session) = parse(SUCCESS).expect("parse");
        assert_eq!(session, "sess-42");
        assert!((reply.spend.cost_usd.expect("cost") - 0.1234).abs() < f64::EPSILON);
        assert_eq!(reply.spend.turns, Some(3));
        assert_eq!(reply.spend.duration_ms, Some(45000));
        assert_eq!(reply.spend.tokens.input, Some(100));
        assert_eq!(reply.spend.tokens.output, Some(20));
        assert_eq!(reply.spend.session.as_deref(), Some("sess-42"));
        assert!(reply.text.contains("voici ce que j'ai fait"));
        // AGENT_LOOP_OK n'est pas un arrêt.
        assert!(reply.stop_line.is_none());
    }

    #[test]
    fn a_stop_marker_in_the_text_becomes_the_stop_line() {
        let json = r#"{"is_error":false,"result":"AGENT_LOOP_STOP: le SPEC est vide",
                       "session_id":"s","total_cost_usd":0.01}"#;
        let (reply, _) = parse(json).expect("parse");
        assert_eq!(
            reply.stop_line.as_deref(),
            Some("AGENT_LOOP_STOP: le SPEC est vide")
        );
    }

    #[test]
    fn a_turn_with_no_cost_field_parses_and_reports_none() {
        // Un porteur, ou une version, qui ne rend pas le coût : le tour reste
        // valide. C'est pour ça que `Reply::cost` est un Option.
        let json = r#"{"is_error":false,"result":"fait","session_id":"s"}"#;
        let (reply, _) = parse(json).expect("parse");
        assert!(reply.spend.cost_usd.is_none());
        // Rien d'observé, et surtout pas des zéros : un registre doit pouvoir
        // écrire « non mesuré » plutôt qu'une session gratuite.
        assert!(reply.spend.is_blind());
    }

    #[test]
    fn an_empty_result_is_a_failure_not_an_empty_success() {
        let json = r#"{"is_error":false,"result":"   ","session_id":"s"}"#;
        let err = parse(json).expect_err("doit échouer");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[test]
    fn unreadable_output_fails_rather_than_panicking() {
        let err = parse("ceci n'est pas du json").expect_err("doit échouer");
        assert!(matches!(err, Halt::Failed(_)));
    }

    // --- classement des échecs ---------------------------------------------

    #[test]
    fn an_exhausted_window_is_a_quota_not_a_failure() {
        let json = r#"{"is_error":true,"subtype":"error_during_execution",
                       "result":"Claude usage limit reached","session_id":"s"}"#;
        let err = parse(json).expect_err("doit échouer");
        assert!(
            matches!(err, Halt::Quota(_)),
            "un quota relance le même travail plus tard ; un échec le rejoue"
        );
    }

    #[test]
    fn any_other_error_subtype_is_a_plain_failure() {
        let json = r#"{"is_error":true,"subtype":"error_during_execution",
                       "result":"the tool crashed","session_id":"s"}"#;
        assert!(matches!(
            parse(json).expect_err("doit échouer"),
            Halt::Failed(_)
        ));
    }

    #[test]
    fn quota_phrases_are_matched_case_insensitively() {
        assert!(matches!(
            halt_for("", "Rate Limit Exceeded"),
            Halt::Quota(_)
        ));
    }

    // --- la fabrique --------------------------------------------------------

    #[tokio::test]
    async fn opening_a_session_spawns_nothing() {
        // Sous C, `open` est gratuit : la conversation naît au premier `ask`.
        // Ce test passe donc sans qu'aucun binaire `claude` existe.
        let factory = ClaudeCliFactory::new(Path::new("/tmp/workspace"), "bypassPermissions");
        let spec = SessionSpec {
            model: "sonnet".to_string(),
            effort: "high".to_string(),
        };
        assert!(factory.open(&spec).await.is_ok());
    }

    /// Le seul test qui dépense vraiment de l'argent, et qui touche le vrai
    /// binaire. Ignoré par défaut — les tests hermétiques ci-dessus tournent
    /// contre une réponse figée, et une réponse figée peut mentir le jour où
    /// Claude Code renomme un champ. Celui-ci est là pour fermer cet écart,
    /// à la demande :
    ///
    /// ```text
    /// cargo test -p harness-core -- --ignored live_
    /// ```
    #[tokio::test]
    #[ignore = "appelle le vrai binaire claude et dépense du quota"]
    async fn live_two_turns_share_one_session() {
        let factory = ClaudeCliFactory::new(Path::new("."), "bypassPermissions");
        let spec = SessionSpec {
            model: "haiku".to_string(),
            effort: "low".to_string(),
        };
        let mut session = factory.open(&spec).await.expect("open");

        let first = session
            .ask("Réponds exactement: un")
            .await
            .expect("1er tour");
        assert!(
            first.spend.cost_usd.is_some(),
            "le premier tour doit rendre un coût"
        );

        // Le second tour doit voir le premier : s'il ne le voit pas, `--resume`
        // n'a pas pris, et toute la proposition C est fausse.
        let second = session
            .ask("Quel mot venais-tu de répondre ?")
            .await
            .expect("2e tour");
        assert!(
            second.text.to_lowercase().contains("un"),
            "le 2e tour n'a pas vu le 1er — --resume n'a pas fonctionné : {}",
            second.text
        );
    }
}
