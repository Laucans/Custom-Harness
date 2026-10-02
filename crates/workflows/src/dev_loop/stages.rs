//! La séquence du round, et tout ce qu'on sait de chaque stage.
//!
//! **La surface de design du workflow, et la seule.** L'ordre des entrées de
//! [`table`] *est* l'ordre d'exécution — le round parcourt ce `Vec`, donc en
//! réordonner deux lignes réordonne le round. Ajouter une entrée ajoute un
//! stage, en retirer une le retire.
//!
//! Ça n'a pas toujours été vrai. La séquence a vécu dans les décorateurs
//! `@listen` d'un graphe, et la table n'était qu'un annuaire de réglages
//! indexé par nom de skill : on pouvait l'inverser entièrement sans que le
//! round change d'un iota, et le test qui vérifiait l'ordre d'exécution passait
//! quand même.
//!
//! Une entrée porte ce qu'on veut savoir d'un stage sans ouvrir un autre
//! fichier — **qui le fait tourner** (modèle, effort), **ce qu'il dit** (les
//! consignes), **ce qui le fait sauter**, **ce qu'il exige**, et **ce qu'il doit
//! avoir obtenu**.
//!
//! # Les étiquettes citées dans la prose
//!
//! Les consignes **nomment** des étiquettes — « ouvre une issue
//! `harness:human` », « l'issue restera sous `harness:waiting-merge` ». Côté
//! Python elles étaient écrites en dur dans le texte, à côté des constantes que
//! le code lisait : le renommage `pipeline:*` → `harness:*` aurait laissé les
//! prompts demander des étiquettes qui n'existent plus, et une session aurait
//! obéi en en créant de nouvelles. Elles sont donc **épissées depuis
//! [`labels`]**, une passe au montage de la table. Un futur renommage ne peut
//! plus désynchroniser le prompt et le code.
//!
//! # Les modèles
//!
//! Répartis selon l'endroit où une mauvaise réponse se paie deux fois.
//! `/business-analyst` écrit le SPEC — le corps de l'issue — que tous les
//! stages suivants lisent ; `code` planifie puis écrit le changement lui-même :
//! une erreur y revient en retravail. `/create-test` écrit du Vitest hermétique
//! contre un spec qui existe déjà, ce qui n'est pas un problème de raisonnement.
//! Se régler sur le registre de coûts, pas sur l'intuition.
//!
//! Il n'y a plus de stage d'archivage. Une task se fermait autrefois en
//! déplaçant deux fichiers ; elle se ferme maintenant parce que la PR de `/code`
//! a mergé avec `Closes #N`.

use std::rc::Rc;

use harness_core::domain::prompts::splice;
use harness_core::execution::{
    Gate, InThisRun, MarkDone, SessionAction, Settings, Stage, StageAlreadyDone, StageBody, Unpaid,
    Verification,
};

use crate::common::labels;
use crate::dev_loop::actions::{Ask, RecordSpecWritten};
use crate::dev_loop::gates;
use crate::dev_loop::state::Loop;
use crate::dev_loop::wiring::Wiring;

const BUSINESS_ANALYST: &str =
    "Take issue #{num} (\"{title}\") — its body is below, under SCOPE. The
interview in step 3 of the skill cannot happen — no human is reachable.
Answer each question you would have asked from docs/PROJECT.md,
docs/ARCHITECTURE.md and the repo itself, and record every answer you had
to assume under an `## Assumptions (autonomous run)` heading. Write the SPEC
into the body of issue #{num} itself — that body is what the /code stage
reads, and nothing else is. Anything a human must do first becomes its own
`{human}` issue, a sub-issue of milestone #{milestone}, declared as a
dependency of #{num}: the loop will not pick #{num} up again until it is
closed. If an assumption would make the task useless or harmful when wrong —
a paid service, a schema decision the later tasks depend on, a credential
only the human holds — that is ambiguity, not a default: AGENT_LOOP_STOP
instead of guessing.";

const CODE: &str = "The task is issue #{num} (\"{title}\"); its body, below under SCOPE, is the
SPEC. Plan it as /tech-analyst: the pre-flight gate, the ordered checklist
against the real code, the stop line, the risks. Then, in this same session,
without waiting for a go-ahead and without /clear, carry out
.claude/skills/code/SKILL.md against your own plan — build, run every
Verification bullet with real output, /code-review, then branch -> PR ->
gh pr merge --rebase. The PR body MUST carry the line `Closes #{num}` on its
own: the loop reads that line off the merged PR to confirm the task shipped,
and without it the round stops rather than replay a task nothing marks as
delivered. The issue will stay open under `{waiting_merge}` until a
human merges the integration branch — that is expected, not a failure. Do
not close the issue yourself.
/code's steps 1-3 are what you just did as the tech analyst; adopt your own
findings instead of re-deriving them. Nothing outside this session can read
your plan, so a gate finding or a risk you do not act on now is lost — put it
in your reply.";

const PLANNER: &str = "Every `{agent}` issue of milestone #{milestone} is closed. Take the
open `{roadmap}` issue with the lowest number — do not ask which one.
Open one `{milestone_label}` issue for it, as a sub-issue of that roadmap
issue, and one `{agent}` sub-issue per task slice, each blocked_by the
one before it. Then close milestone #{milestone}, and close the roadmap issue
it hangs off if that item is now fully delivered: the loop works on the
LOWEST-numbered open milestone, so leaving the finished one open makes every
later round roll over again instead of picking up what you just planned. Do
not add `{ready}` to anything: the human opens the tap. Step 3 of the
skill applies in full: verify the ground truth in the repo, and if a
dependency the item builds on is not actually there, AGENT_LOOP_STOP with
what is missing rather than planning on top of it.";

/// Les consignes d'un stage, étiquettes épissées depuis [`labels`].
///
/// Une seule passe, et avant celle de la portée : les valeurs insérées sont des
/// constantes du code, donc sans accolade, et les `{num}`/`{title}`/`{milestone}`
/// restent littéraux pour que `prompts::extra_for` les remplisse ensuite.
fn with_labels(text: &str) -> String {
    splice(
        text,
        &[
            ("agent", labels::AGENT),
            ("human", labels::HUMAN),
            ("ready", labels::READY),
            ("roadmap", labels::ROADMAP),
            ("milestone_label", labels::MILESTONE),
            ("waiting_merge", labels::WAITING_MERGE),
        ],
    )
}

/// Ce que tout stage subit avant de payer : le filtre, et la reprise.
fn always(stage: &str) -> Vec<Box<dyn Verification<Loop>>> {
    vec![
        Box::new(InThisRun {
            stage: stage.to_string(),
        }),
        Box::new(StageAlreadyDone {
            stage: stage.to_string(),
        }),
    ]
}

/// Ce qu'un stage payant demande à sa session, et ce qu'il fait ensuite.
///
/// `then` est le travail local qui suit la session — relire, étiqueter. Il
/// vient **avant** `MarkDone` : se déclarer fait avant d'avoir consigné
/// laisserait, sur un run interrompu entre les deux, un stage marqué fait dont
/// rien n'a été enregistré.
struct Paid<'a> {
    stage: &'a str,
    model: &'a str,
    lead: &'a str,
    instructions: &'a str,
    scoped: bool,
    then: Vec<Box<dyn SessionAction<Loop>>>,
}

impl Paid<'_> {
    fn body(self, wiring: &Wiring, turn: u32) -> StageBody<Loop> {
        let mut actions: Vec<Box<dyn SessionAction<Loop>>> = vec![Box::new(Ask {
            stage: self.stage.to_string(),
            lead: self.lead.to_string(),
            instructions: with_labels(self.instructions),
            scoped: self.scoped,
            round: turn,
            branch: wiring.integration_branch.clone(),
            injector: wiring.injector(),
            spending: Rc::clone(&wiring.spending),
        })];
        actions.extend(self.then);
        actions.push(Box::new(Unpaid(MarkDone {
            stage: self.stage.to_string(),
        })));
        StageBody::Session {
            spec: wiring.spec(self.model, "high"),
            sessions: Rc::clone(&wiring.sessions),
            actions,
        }
    }
}

/// `/business-analyst` : écrit le SPEC dans le corps de l'issue.
#[must_use]
pub fn business_analyst(wiring: &Wiring, turn: u32) -> Stage<Loop> {
    let body = Paid {
        stage: "business-analyst",
        model: "opus",
        lead: "/business-analyst",
        instructions: BUSINESS_ANALYST,
        scoped: true,
        // Consigner le SPEC relu est du travail local, et il va *dans* la
        // stage — pas dans une garde, qui n'aurait pas le droit d'écrire.
        then: vec![Box::new(Unpaid(RecordSpecWritten {
            gh: Rc::clone(&wiring.gh),
        }))],
    }
    .body(wiring, turn);
    let mut pre = always("business-analyst");
    pre.push(Box::new(gates::SpecAlreadyWritten));
    Stage {
        name: "business-analyst".to_string(),
        pre: Some(Gate {
            name: "business-analyst requiert",
            checks: pre,
        }),
        post: Some(Gate {
            name: "business-analyst doit obtenir",
            checks: vec![Box::new(gates::IssueBodyIsNotEmpty {
                gh: Rc::clone(&wiring.gh),
            })],
        }),
        body,
    }
}

/// `code` : planifie en `/tech-analyst`, puis construit et livre.
///
/// Un seul stage pour deux skills, et une seule session : `/code` adopte le
/// plan que la session vient d'écrire. Rien en dehors de la session ne peut le
/// relire, donc le découper en deux stages le perdrait.
#[must_use]
pub fn code(wiring: &Wiring, turn: u32) -> Stage<Loop> {
    let mut pre = always("code");
    pre.push(Box::new(gates::CodeAlreadyDelivered {
        gh: Rc::clone(&wiring.gh),
        integration_branch: wiring.integration_branch.clone(),
        restart: wiring.restart,
    }));
    pre.push(Box::new(gates::CodeHasASpec));
    Stage {
        name: "code".to_string(),
        pre: Some(Gate {
            name: "code requiert",
            checks: pre,
        }),
        // Rien à vérifier après : ce que `code` doit avoir obtenu est la
        // post-condition du *round* — une PR mergée qui porte `Closes #N` —, et
        // la vérifier deux fois ne dirait rien de plus.
        post: None,
        body: Paid {
            stage: "code",
            model: "opus",
            lead: "/tech-analyst",
            instructions: CODE,
            scoped: true,
            then: Vec::new(),
        }
        .body(wiring, turn),
    }
}

/// `/create-test` : du Vitest hermétique contre un spec déjà écrit.
///
/// Ni consignes propres ni garde : il travaille contre un spec existant, le
/// préambule et la portée lui suffisent, et rien de ce qu'il produit ne
/// conditionne la suite — c'est le dernier stage.
#[must_use]
pub fn create_test(wiring: &Wiring, turn: u32) -> Stage<Loop> {
    Stage {
        name: "create-test".to_string(),
        pre: Some(Gate {
            name: "create-test requiert",
            checks: always("create-test"),
        }),
        post: None,
        body: Paid {
            stage: "create-test",
            model: "sonnet",
            lead: "/create-test",
            instructions: "",
            scoped: true,
            then: Vec::new(),
        }
        .body(wiring, turn),
    }
}

/// `/planner` : ouvre l'item de roadmap suivant quand le milestone est fini.
///
/// **Pas dans [`table`]**, et c'est voulu : le rollover est une branche, pas un
/// stage de plus — il part quand il n'y a **aucune** task, donc quand la
/// séquence n'a rien à faire. Le lanceur décide de le brancher ou non ;
/// enchaîner en non surveillé dépense un run opus et engage le projet sur un
/// item de roadmap que personne n'a lu.
#[must_use]
pub fn planner(wiring: &Wiring, turn: u32) -> Stage<Loop> {
    Stage {
        name: "planner".to_string(),
        pre: Some(Gate {
            name: "planner requiert",
            checks: always("planner"),
        }),
        post: Some(Gate {
            name: "planner doit obtenir",
            checks: vec![Box::new(gates::PlannerOpenedATask {
                gh: Rc::clone(&wiring.gh),
            })],
        }),
        body: Paid {
            stage: "planner",
            model: "opus",
            lead: "/planner",
            instructions: PLANNER,
            // Le seul stage dans ce cas : il ne travaille sur aucune task, il
            // en ouvre.
            scoped: false,
            then: Vec::new(),
        }
        .body(wiring, turn),
    }
}

/// La séquence d'un round, dans l'ordre d'exécution.
#[must_use]
pub fn table(wiring: &Wiring, turn: u32) -> Vec<Stage<Loop>> {
    vec![
        business_analyst(wiring, turn),
        code(wiring, turn),
        create_test(wiring, turn),
    ]
}

/// `business-analyst(opus/high) -> code(opus/high) -> …`
///
/// Dérivée des stages **construites**, pas d'une seconde liste : la ligne
/// annoncée et ce qui tourne ne peuvent pas diverger, et le forçage
/// `MODEL`/`EFFORT` y apparaît sans être réappliqué ici.
#[must_use]
pub fn summary(stages: &[Stage<Loop>], settings: &Settings) -> String {
    stages
        .iter()
        .filter(|stage| settings.runs(&stage.name))
        .map(|stage| match &stage.body {
            StageBody::Session { spec, .. } => {
                format!("{}({}/{})", stage.name, spec.model, spec.effort)
            }
            StageBody::Local { .. } => format!("{}(local)", stage.name),
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// Les stages que `--stages` laisse de côté — jamais en silence.
#[must_use]
pub fn filtered_out(stages: &[Stage<Loop>], settings: &Settings) -> Vec<String> {
    stages
        .iter()
        .filter(|stage| !settings.runs(&stage.name))
        .map(|stage| stage.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_loop::wiring::fake;

    fn settings(stages: &str) -> Settings {
        Settings {
            dry_run: false,
            stages: stages.to_string(),
        }
    }

    #[test]
    fn the_order_of_the_table_is_the_order_of_the_round() {
        let names: Vec<String> = table(&fake::wiring(), 1)
            .iter()
            .map(|stage| stage.name.clone())
            .collect();
        assert_eq!(names, ["business-analyst", "code", "create-test"]);
    }

    #[test]
    fn no_instruction_text_names_a_label_the_code_does_not_use() {
        // Le mode de panne que ça évite : le prompt demande `pipeline:human`,
        // l'étiquette n'existe plus, et la session en crée une.
        for text in [BUSINESS_ANALYST, CODE, PLANNER] {
            let said = with_labels(text);
            assert!(
                !said.contains("pipeline:"),
                "une étiquette `pipeline:*` reste dans la prose"
            );
            assert!(
                !said.contains("{human}") && !said.contains("{agent}"),
                "un gabarit d'étiquette n'a pas été épissé"
            );
        }
    }

    #[test]
    fn every_label_the_prose_mentions_is_one_of_the_seven() {
        let said = format!(
            "{} {} {}",
            with_labels(BUSINESS_ANALYST),
            with_labels(CODE),
            with_labels(PLANNER)
        );
        for label in labels::LOOP {
            // `spec-written` n'est pas citée : c'est le harness qui la pose,
            // pas la session. Les autres doivent apparaître telles quelles.
            if label == labels::SPEC_WRITTEN {
                continue;
            }
            assert!(said.contains(label), "{label} absente de la prose");
        }
    }

    #[test]
    fn the_task_placeholders_survive_the_label_pass_for_the_scope_to_fill() {
        let said = with_labels(BUSINESS_ANALYST);
        assert!(said.contains("#{num}"), "le numéro reste à remplir");
        assert!(said.contains("{title}"));
        assert!(said.contains("milestone #{milestone}"));
    }

    #[test]
    fn business_analyst_records_the_spec_before_declaring_itself_done() {
        // L'ordre compte : se déclarer fait avant d'avoir consigné laisserait
        // un SPEC écrit mais non étiqueté, donc réécrit au run suivant.
        let stage = business_analyst(&fake::wiring(), 1);
        let StageBody::Session { actions, .. } = &stage.body else {
            panic!("une stage payante");
        };
        assert_eq!(actions.len(), 3, "Ask, consigner, marquer");
    }

    #[test]
    fn the_planner_is_not_in_the_sequence_because_rollover_is_a_branch() {
        assert!(
            !table(&fake::wiring(), 1)
                .iter()
                .any(|stage| stage.name == "planner")
        );
    }

    #[test]
    fn the_summary_reads_the_models_off_the_stages_that_will_run() {
        let said = summary(&table(&fake::wiring(), 1), &settings(""));
        assert_eq!(
            said,
            "business-analyst(opus/high) -> code(opus/high) -> create-test(sonnet/high)"
        );
    }

    #[test]
    fn a_forced_model_shows_up_in_the_summary_without_being_reapplied() {
        let mut wiring = fake::wiring();
        wiring.model = "sonnet".to_string();
        let said = summary(&table(&wiring, 1), &settings(""));
        assert!(!said.contains("opus"), "le forçage vaut pour tout le run");
    }

    #[test]
    fn what_stages_leaves_out_is_named_rather_than_silently_dropped() {
        let built = table(&fake::wiring(), 1);
        let cfg = settings("code");
        assert_eq!(summary(&built, &cfg), "code(opus/high)");
        assert_eq!(
            filtered_out(&built, &cfg),
            ["business-analyst".to_string(), "create-test".to_string()]
        );
    }
}
