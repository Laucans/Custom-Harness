//! Ce que le round **fait** : choisir, demander, consigner, marquer.
//!
//! Le pendant de `gates.rs`, et la raison pour laquelle les deux fichiers
//! existent séparément. La décision n°1 dit qu'une `Verification` juge et
//! n'écrit pas ; tout ce qui écrit est donc ici, y compris les moitiés
//! « écrire » des deux gardes hybrides du Python :
//!
//! | Python | sa moitié qui juge | sa moitié qui fait |
//! | --- | --- | --- |
//! | `spec_is_in_the_issue` | `gates::IssueBodyIsNotEmpty` | [`RecordSpecWritten`] |
//! | `task_is_delivered` | `gates::AMergedPrClosesTheTask` | [`MarkWaitingMerge`] |
//!
//! La scission coûte une relecture d'issue par paire : l'action relit pour
//! écrire, la garde relit pour juger. C'est le prix assumé de « juger n'est pas
//! faire » — un appel local gratuit contre une session qui coûte des dollars.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::spending::{Entry, Spending};
use harness_core::domain::{Halt, Named, Outcome, Scoped, Verdict, markers, prompts};
use harness_core::execution::{Action, Context, Open, SessionAction};

use crate::common::labels;
use crate::dev_loop::state::Loop;
use crate::dev_loop::{board, tasks};

/// Le numéro de la task en cours, ou l'échec de le lire.
fn number_of(ctx: &Context<Loop>) -> Outcome<u64> {
    ctx.state.task.number.parse().map_err(|_| {
        Halt::Failed(format!(
            "numéro de task illisible : {:?}",
            ctx.state.task.number
        ))
    })
}

/// Choisit la task du round, ou bascule en rollover.
///
/// La première action de tout round, et la seule qui ait le droit de ne rien
/// trouver. Ses trois sorties sont l'`if/elif/else` qui a remplacé le routeur
/// à trois branches d'un moteur de graphe :
///
/// - une task (celle de la reprise, sinon celle du tableau) → le round continue ;
/// - aucune task jouable **mais des tasks ouvertes** → arrêt, en nommant le
///   geste qui débloque. Surtout pas un rollover : confondre les deux fait
///   payer un `/planner` pour une case `harness:ready` que personne n'a cochée ;
/// - aucune task ouverte du tout → rollover.
pub struct PickTask {
    /// De quoi lire le tableau.
    pub gh: Rc<dyn GitHub>,
    /// La task que le point de reprise désigne, s'il y en a un.
    ///
    /// Elle **prime** sur le choix du tableau : un round interrompu après le
    /// merge de `/code` travaille sur une issue déjà fermée, que plus rien
    /// n'offrirait, et `/create-test` serait perdu.
    pub resuming: Option<String>,
}

#[async_trait(?Send)]
impl Action<Loop> for PickTask {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        // Une seule lecture du tableau par round. Le lanceur a lu le pointeur
        // de reprise, pas le tableau : il n'y a donc pas deux lectures qui
        // pourraient ne pas répondre la même chose.
        let here = board::read(self.gh.as_ref()).await?;
        ctx.state.milestone = Named {
            number: here.milestone.number.to_string(),
            title: here.milestone.title.clone(),
            body: here.milestone.body.clone(),
        };
        ctx.traces.say(&format!(
            "milestone {}: {}",
            here.milestone.reference(),
            here.milestone.title
        ));
        let resumed = self.resuming.as_deref().and_then(|key| here.find(key));
        let Some(task) = resumed.or_else(|| here.next()) else {
            if !here.open_agents().is_empty() {
                return Err(Halt::Halted(here.stuck()));
            }
            ctx.state.rollover = true;
            ctx.traces.say(&format!(
                "milestone {} has no open {} sub-issue — opening the next \
                 roadmap item",
                here.milestone.reference(),
                labels::AGENT
            ));
            return Ok(Verdict::Continue);
        };
        ctx.state.task = Named {
            number: task.number.to_string(),
            title: task.title.clone(),
            body: task.body.clone(),
        };
        ctx.state.task_key = task.key();
        ctx.state.kind = tasks::kind(task).to_string();
        ctx.state.spec_written = tasks::spec_written(task);
        ctx.state.resumed = resumed.is_some();
        ctx.traces.say(&format!(
            "task {}: {} [{}]",
            task.reference(),
            task.title,
            tasks::kind(task)
        ));
        Ok(Verdict::Continue)
    }
}

/// Consigne que le SPEC est dans le corps de l'issue.
///
/// La moitié « écrire » de l'ex-`spec_is_in_the_issue` : elle relit le corps —
/// qui **est** le SPEC, et que le stage suivant recevra dans sa portée —, le
/// recopie dans l'état, et pose l'étiquette.
///
/// Elle ne juge pas. Un corps vide n'est pas une erreur ici, c'est simplement
/// qu'il n'y a rien à consigner : elle n'étiquette pas et se taît, et c'est la
/// garde qui suit — `gates::IssueBodyIsNotEmpty` — qui arrête le round en le
/// disant. Poser l'étiquette d'abord ferait sauter la rédaction au run suivant
/// alors que rien n'a été écrit.
pub struct RecordSpecWritten {
    /// De quoi relire l'issue et l'étiqueter.
    pub gh: Rc<dyn GitHub>,
}

#[async_trait(?Send)]
impl Action<Loop> for RecordSpecWritten {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let number = number_of(ctx)?;
        let issue = self.gh.issue(number).await?;
        if issue.body.trim().is_empty() {
            return Ok(Verdict::Continue);
        }
        ctx.state.task.body = issue.body;
        self.gh.add_label(number, labels::SPEC_WRITTEN).await?;
        ctx.state.spec_written = true;
        Ok(Verdict::Continue)
    }
}

/// Marque la task livrée, quand une PR mergée le prouve.
///
/// La moitié « écrire » de l'ex-`task_is_delivered`, et la dernière chose qu'un
/// round fait.
///
/// **Livrée n'est pas fermée, et l'écart est voulu.** `Closes #N` ne ferme rien
/// ici : GitHub ne ferme une issue liée qu'au merge dans la branche **par
/// défaut**, et la boucle merge dans la branche d'intégration. Plutôt que de
/// fermer à sa place — ce qui dirait « intégré dans `main` » d'un travail qui
/// n'y est pas — le round pose `harness:waiting-merge`. L'issue reste ouverte,
/// ne sera plus jamais choisie, et ne bloque plus la suivante ; c'est l'humain
/// qui la ferme en fusionnant.
///
/// Sans preuve, elle ne marque rien et se taît : c'est la garde qui dit pourquoi
/// le round s'arrête.
pub struct MarkWaitingMerge {
    /// De quoi lire l'issue et les PR, et poser l'étiquette.
    pub gh: Rc<dyn GitHub>,
    /// La branche sur laquelle la preuve est cherchée.
    pub integration_branch: String,
}

#[async_trait(?Send)]
impl Action<Loop> for MarkWaitingMerge {
    async fn run(&self, ctx: &mut Context<Loop>) -> Outcome<Verdict> {
        if ctx.settings.dry_run || !ctx.state.has_task() {
            return Ok(Verdict::Continue);
        }
        let number = number_of(ctx)?;
        let here = self.gh.issue(number).await?;
        // Déjà fermée par GitHub — le jour où la branche d'intégration devient
        // la branche par défaut — ou déjà marquée : rien à reposer.
        if here.is_closed() || tasks::waiting_merge(&here) {
            return Ok(Verdict::Continue);
        }
        let merged = self.gh.merged_prs(&self.integration_branch).await?;
        let Some(shipped) = tasks::first_closing(&merged, number) else {
            return Ok(Verdict::Continue);
        };
        ctx.traces.say(&format!(
            "#{number} livrée par la PR {}, mergée sur {} — marquée {}, à vous \
             de la fermer en fusionnant dans la branche par défaut",
            shipped.reference(),
            self.integration_branch,
            labels::WAITING_MERGE
        ));
        self.gh.add_label(number, labels::WAITING_MERGE).await?;
        Ok(Verdict::Continue)
    }
}

/// Une commande envoyée dans la session ouverte du stage.
///
/// La seule action qui dépense, et donc la seule qui consigne une ligne de
/// registre. Elle compose le prompt — commande, préambule, consignes, portée —,
/// l'envoie, relit les deux marqueurs, et traduit ce qui revient.
///
/// Plusieurs `Ask` dans une même stage sont le cas normal : côté Python,
/// `StageSpec.lead` servait à coudre `/tech-analyst` et `/code` dans un seul
/// prompt parce qu'une stage ne pouvait parler qu'une fois.
pub struct Ask {
    /// Le nom du stage, pour le journal et la colonne `stage` du registre.
    pub stage: String,
    /// La commande qui ouvre le prompt (`/code`, `/tech-analyst`, …).
    pub lead: String,
    /// Les consignes propres à ce stage. Vide : le préambule et la portée
    /// suffisent, comme pour `/create-test`.
    pub instructions: String,
    /// Injecter le bloc SCOPE.
    ///
    /// Faux pour `/planner` et pour lui seul : il ne travaille sur aucune task,
    /// il en ouvre. Lui injecter un bloc ISSUE vide lui donnerait une task à
    /// chercher.
    pub scoped: bool,
    /// Le numéro de round, pour la colonne `round` du registre.
    pub round: u32,
    /// La branche d'intégration citée dans le préambule.
    pub branch: String,
    /// Le nom cité comme injecteur.
    pub injector: &'static str,
    /// Où la dépense est consignée.
    pub spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<Loop> for Ask {
    async fn run(&self, open: &mut Open<'_, Loop>) -> Outcome<Verdict> {
        let scope = open.state.scope();
        let extra = prompts::extra_for(
            &self.instructions,
            if self.scoped { Some(&scope) } else { None },
            self.injector,
        );
        let prompt = prompts::build(&self.lead, &self.branch, &extra, self.injector);
        let log = open.traces.bind(&self.stage);
        let dry_run = open.settings.dry_run;
        let task = open.state.task_key.clone();

        let asked = open.session.ask(&prompt).await;
        // Consignée **avant** de propager quoi que ce soit : un stage mort est
        // justement celui dont on veut la ligne. Un échec n'a pas de `Spend` —
        // l'adaptateur l'a traduit en `Halt` — donc la ligne dit « rien
        // d'observé », ce qui est la vérité, et non une colonne de zéros.
        if !dry_run {
            let blind = harness_core::domain::Spend::default();
            let (spend, outcome) = match &asked {
                Ok(reply) => (&reply.spend, "ok"),
                Err(halt) => (&blind, halt.prefix()),
            };
            self.spending.record(&Entry {
                round: self.round,
                task: &task,
                stage: &self.stage,
                spend,
                outcome,
            })?;
        }
        let reply = asked?;

        if let Some(stop) = markers::stop_line(&reply.text) {
            // Une session qui répond AGENT_LOOP_STOP s'est arrêtée d'elle-même :
            // un résultat correct, et la raison est la sienne.
            return Err(Halt::Halted(format!("{stop} (/{})", self.stage)));
        }
        match markers::ok_line(&reply.text) {
            Some(ok) => log.say(&ok),
            None if !dry_run => log.say(&format!(
                "/{} produced no {} marker — relying on the structural checks",
                self.stage,
                markers::OK
            )),
            None => {}
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use crate::dev_loop::state::Loop;
    use harness_core::adapters::agent::{Reply, Session};
    use harness_core::domain::{Issue, Spend};
    use harness_core::execution::Settings;
    use harness_core::traces::{Logbook, Sink, Verbosity};
    use std::cell::RefCell;

    fn ctx(state: Loop) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            state,
            Logbook::null(),
        )
    }

    fn issue(number: u64, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("task {number}"),
            state: "open".to_string(),
            labels: labels.iter().map(|l| (*l).to_string()).collect(),
            ..Issue::default()
        }
    }

    fn milestone(number: u64) -> Issue {
        issue(number, &[labels::MILESTONE])
    }

    fn task(number: u64) -> Issue {
        issue(number, &[labels::AGENT, labels::READY])
    }

    // --- PickTask ----------------------------------------------------------

    #[tokio::test]
    async fn picking_fills_the_scope_so_no_session_starts_blind() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![task(11)])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask { gh, resuming: None }
            .run(&mut context)
            .await
            .expect("une task");
        assert_eq!(context.state.milestone.number, "4");
        assert_eq!(context.state.task.number, "11");
        assert_eq!(context.state.task_key, "11");
        assert_eq!(context.state.kind, "auto");
        assert!(!context.state.rollover);
    }

    #[tokio::test]
    async fn the_resume_key_wins_over_what_the_board_would_offer() {
        // Le cas réel : /code a mergé, l'issue est fermée, et /create-test
        // reste à faire. Rien ne l'offrirait.
        let mut shipped = task(11);
        shipped.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![shipped, task(12)])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask {
            gh,
            resuming: Some("11".to_string()),
        }
        .run(&mut context)
        .await
        .expect("une task");
        assert_eq!(context.state.task_key, "11");
        assert!(context.state.resumed);
    }

    #[tokio::test]
    async fn no_open_task_at_all_is_a_rollover() {
        let mut done = task(11);
        done.state = "closed".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![done])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        PickTask { gh, resuming: None }
            .run(&mut context)
            .await
            .expect("un rollover");
        assert!(context.state.rollover);
        assert!(!context.state.has_task());
    }

    #[tokio::test]
    async fn open_but_unplayable_tasks_halt_rather_than_roll_over() {
        // Confondre les deux fait payer un /planner opus pour une case
        // `harness:ready` que personne n'a cochée.
        let gh = Rc::new(FakeGitHub {
            issues: vec![milestone(4)],
            subs: vec![(4, vec![issue(11, &[labels::AGENT])])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        let err = PickTask { gh, resuming: None }
            .run(&mut context)
            .await
            .expect_err("doit s'arrêter");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(!context.state.rollover, "surtout pas un rollover");
        assert!(err.reason().contains(labels::READY));
    }

    #[tokio::test]
    async fn an_unreadable_board_never_becomes_a_rollover() {
        let gh = Rc::new(FakeGitHub {
            broken: Some(Halt::Unreadable("jeton expiré".to_string())),
            ..FakeGitHub::default()
        });
        let mut context = ctx(Loop::default());
        let err = PickTask { gh, resuming: None }
            .run(&mut context)
            .await
            .expect_err("doit échouer");
        assert!(matches!(err, Halt::Unreadable(_)));
        assert!(!context.state.rollover);
    }

    // --- RecordSpecWritten -------------------------------------------------

    fn with_task(number: &str) -> Loop {
        Loop {
            task: Named {
                number: number.to_string(),
                title: "une task".to_string(),
                body: String::new(),
            },
            task_key: number.to_string(),
            ..Loop::default()
        }
    }

    #[tokio::test]
    async fn recording_the_spec_copies_the_reread_body_into_the_scope() {
        let mut written = issue(11, &[labels::AGENT]);
        written.body = "le SPEC, écrit par /business-analyst".to_string();
        let gh = Rc::new(FakeGitHub {
            issues: vec![written],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        RecordSpecWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("consigné");
        assert!(context.state.task.body.contains("le SPEC"));
        assert!(context.state.spec_written);
        assert_eq!(
            gh.writes(),
            vec![Wrote::Label(11, labels::SPEC_WRITTEN.to_string())]
        );
    }

    #[tokio::test]
    async fn an_empty_body_is_not_labelled_as_written() {
        // Poser l'étiquette ferait sauter la rédaction au run suivant alors
        // que rien n'a été écrit. C'est la garde qui arrête, pas celle-ci.
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        RecordSpecWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("rien à consigner");
        assert!(!context.state.spec_written);
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn a_dry_run_writes_nothing_to_github() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        context.settings.dry_run = true;
        RecordSpecWritten { gh: gh.clone() }
            .run(&mut context)
            .await
            .expect("rien");
        assert!(gh.writes().is_empty());
    }

    // --- MarkWaitingMerge --------------------------------------------------

    fn delivered_by(pr: u64, closes: u64) -> Issue {
        Issue {
            number: pr,
            body: format!("Closes #{closes}"),
            ..Issue::default()
        }
    }

    #[tokio::test]
    async fn a_merged_pr_marks_the_task_waiting_merge_without_closing_it() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            merged: vec![delivered_by(99, 11)],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("marquée");
        // Étiquetée, jamais fermée : fermer dirait « intégré dans main ».
        assert_eq!(
            gh.writes(),
            vec![Wrote::Label(11, labels::WAITING_MERGE.to_string())]
        );
    }

    #[tokio::test]
    async fn without_proof_it_marks_nothing_and_leaves_the_gate_to_speak() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT])],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("rien à marquer");
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn an_already_marked_task_is_not_relabelled() {
        let gh = Rc::new(FakeGitHub {
            issues: vec![issue(11, &[labels::AGENT, labels::WAITING_MERGE])],
            merged: vec![delivered_by(99, 11)],
            ..FakeGitHub::default()
        });
        let mut context = ctx(with_task("11"));
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("déjà marquée");
        assert!(gh.writes().is_empty());
    }

    #[tokio::test]
    async fn a_rollover_round_has_no_task_to_mark() {
        let gh = Rc::new(FakeGitHub::default());
        let mut context = ctx(Loop {
            rollover: true,
            ..Loop::default()
        });
        MarkWaitingMerge {
            gh: gh.clone(),
            integration_branch: "main_agent".to_string(),
        }
        .run(&mut context)
        .await
        .expect("rien à marquer");
        assert!(gh.writes().is_empty());
    }

    // --- Ask ---------------------------------------------------------------

    struct Scripted {
        reply: Result<Reply, Halt>,
        seen: RefCell<Vec<String>>,
    }

    #[async_trait(?Send)]
    impl Session for Scripted {
        async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
            self.seen.borrow_mut().push(prompt.to_string());
            self.reply.clone()
        }
    }

    #[derive(Default)]
    struct Recorded(RefCell<Vec<(String, String, f64)>>);

    impl Spending for Recorded {
        fn record(&self, entry: &Entry<'_>) -> Outcome<()> {
            self.0.borrow_mut().push((
                entry.stage.to_string(),
                entry.outcome.to_string(),
                entry.spend.cost_usd.unwrap_or(-1.0),
            ));
            Ok(())
        }
    }

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn answered(text: &str) -> Reply {
        Reply {
            text: text.to_string(),
            stop_line: None,
            spend: Spend {
                cost_usd: Some(0.42),
                ..Spend::default()
            },
        }
    }

    fn scoped_state() -> Loop {
        Loop {
            milestone: Named {
                number: "4".to_string(),
                title: "Le chat".to_string(),
                body: "ce que le milestone dit".to_string(),
            },
            task: Named {
                number: "11".to_string(),
                title: "La grille".to_string(),
                body: "le SPEC".to_string(),
            },
            task_key: "11".to_string(),
            ..Loop::default()
        }
    }

    struct Asked {
        verdict: Outcome<Verdict>,
        prompts: Vec<String>,
        rows: Vec<(String, String, f64)>,
        said: String,
    }

    async fn ask_with(ask_scoped: bool, reply: Result<Reply, Halt>, dry_run: bool) -> Asked {
        let spending = Rc::new(Recorded::default());
        let capture = Rc::new(Capture::default());
        let mut context = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            scoped_state(),
            Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal),
        );
        let mut session = Scripted {
            reply,
            seen: RefCell::new(Vec::new()),
        };
        let ask = Ask {
            stage: "code".to_string(),
            lead: "/tech-analyst".to_string(),
            instructions: "la task est #{num} (\"{title}\")".to_string(),
            scoped: ask_scoped,
            round: 3,
            branch: "main_agent".to_string(),
            injector: "test",
            spending: Rc::clone(&spending) as Rc<dyn Spending>,
        };
        let verdict = {
            let mut open = Open {
                ctx: &mut context,
                session: &mut session,
            };
            ask.run(&mut open).await
        };
        Asked {
            verdict,
            prompts: session.seen.borrow().clone(),
            rows: spending.0.borrow().clone(),
            said: capture.0.borrow().join("\n"),
        }
    }

    #[tokio::test]
    async fn the_prompt_carries_the_command_the_preamble_and_the_scope() {
        let run = ask_with(true, Ok(answered("AGENT_LOOP_OK: livré")), false).await;
        let prompt = &run.prompts[0];
        assert!(prompt.starts_with("/tech-analyst\n"));
        assert!(prompt.contains("main_agent"), "la branche d'intégration");
        assert!(prompt.contains("la task est #11 (\"La grille\")"));
        assert!(prompt.contains("ISSUE #11 — La grille"));
        assert!(prompt.contains("MILESTONE #4"));
    }

    #[tokio::test]
    async fn an_unscoped_ask_gets_no_issue_block_to_hunt_for() {
        let run = ask_with(false, Ok(answered("AGENT_LOOP_OK: planifié")), false).await;
        assert!(!run.prompts[0].contains("ISSUE #"));
    }

    #[tokio::test]
    async fn the_ok_line_is_echoed_into_the_journal_under_the_stage_tag() {
        let run = ask_with(true, Ok(answered("AGENT_LOOP_OK: livré")), false).await;
        assert!(run.said.contains("[code] AGENT_LOOP_OK: livré"));
    }

    #[tokio::test]
    async fn a_missing_ok_marker_is_said_rather_than_treated_as_a_failure() {
        let run = ask_with(true, Ok(answered("j'ai fini")), false).await;
        assert!(run.verdict.is_ok());
        assert!(run.said.contains("no AGENT_LOOP_OK marker"));
    }

    #[tokio::test]
    async fn a_stop_marker_halts_and_keeps_the_sessions_own_reason() {
        let run = ask_with(
            true,
            Ok(answered(
                "AGENT_LOOP_STOP: le SPEC suppose un service payant",
            )),
            false,
        )
        .await;
        let err = run.verdict.expect_err("doit s'arrêter");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains("service payant"));
        assert!(err.reason().contains("(/code)"), "nommer le stage");
    }

    #[tokio::test]
    async fn a_successful_ask_records_what_it_cost() {
        let run = ask_with(true, Ok(answered("AGENT_LOOP_OK: livré")), false).await;
        assert_eq!(run.rows, vec![("code".to_string(), "ok".to_string(), 0.42)]);
    }

    #[tokio::test]
    async fn a_dead_stage_still_gets_its_line_and_it_says_nothing_was_observed() {
        // C'est le stage mort dont on veut les traces. Et une ligne à zéro se
        // relirait comme une session gratuite : -1.0 est le témoin du `None`.
        let run = ask_with(true, Err(Halt::Quota("fenêtre épuisée".into())), false).await;
        assert!(matches!(run.verdict, Err(Halt::Quota(_))));
        assert_eq!(
            run.rows,
            vec![("code".to_string(), "QUOTA".to_string(), -1.0)]
        );
    }

    #[tokio::test]
    async fn a_dry_run_records_no_spend_because_none_was_made() {
        let run = ask_with(true, Ok(answered("")), true).await;
        assert!(run.rows.is_empty());
        // Et il ne réclame pas un marqueur à une session que personne n'a
        // ouverte.
        assert!(!run.said.contains("no AGENT_LOOP_OK marker"));
    }
}
