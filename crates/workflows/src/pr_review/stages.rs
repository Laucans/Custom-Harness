//! La revue : la séquence, et ce qu'on sait de chaque étape.
//!
//! **La surface de design du workflow, et la seule.** Deux passes payantes,
//! puis la publication — qui est une étape de la table sans être une
//! session, parce que la séquence enchaîne les deux sortes.
//!
//! Les modèles sont répartis selon ce qu'est le travail. `/code` a déjà fait
//! tourner une revue adverse avant de merger : la passe 1 est un second avis
//! en contexte neuf, pas la seule ligne de défense. La passe 2 résume ce que
//! la passe 1 et le diff disent déjà — c'est de la rédaction, pas une chasse.

use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::agent::SessionSpec;
use harness_core::adapters::shell::github::GitHub;
use harness_core::adapters::store::spending::Spending;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Outcome, Verdict};
use harness_core::execution::{Gate, Open, SessionAction, Stage, StageBody, ask_and_record};

use crate::pr_review::gates::{self, InlinePassIsOff, NothingIsPosted};
use crate::pr_review::publish::Publish;
use crate::pr_review::state::ReviewState;

const BRIEF_PROMPT: &str =
    "You are writing review notes on pull request #{num} (\"{title}\", {head} -> {base})
in this repository. The branch was written by an unattended agent loop: a
human is about to read the diff for the first time and has to decide whether
to trust it. Your notes are the only orientation they get.

Start by reading the change and its intent:
  gh pr diff {num}
  gh pr view {num} --json title,body,commits

You also need the spec the branch was built from. It is the body of the
GitHub issue this PR closes — the PR body carries `Closes #<n>` on its own
line:
  gh issue view <n> --json title,body
Read the milestone issue it hangs under only if you need to place the task.

The spec is not committed alongside the code, so it cannot drift inside the
diff. What is worth checking instead is the gap between what the issue asks
for and what the branch does: scope quietly dropped, or quietly widened, is
the drift this comment exists to surface.

A line-by-line bug hunt already ran and posted its findings inline. Here is
what it reported — reference it, do not repeat it:
<inline-review-output>
{findings}
</inline-review-output>

Write ONE markdown comment body, and output nothing else — no preamble, no
\"here is the comment\", no code fence around the whole thing. Around 200-350
words, these sections, dropping any that would be empty:

**Ce que fait ce lot** — the change in 2-4 sentences, in intent terms, not a
file listing.

**À regarder en priorité** — the 2-4 places where a human's attention is
actually worth spending, each as `file.ts:line` (markdown link relative to
the repo root) plus one line on why. Rank them; do not list everything.

**Écarts avec le SPEC** — anything the spec asked for that is not here, or
here but not asked for. Say \"conforme\" if it matches.

**Hypothèses prises** — decisions the agent made that the spec left open,
and that a human might have made differently. This is the section that most
often matters: the loop guesses silently.

**Questions** — what you would ask the author. Omit if you have none.

Write in French, the way a colleague leaves review notes. Be concrete and
specific to this diff — no generic advice, no praise, no summary of your own
process. If the change is small and clean, say so briefly rather than
inflating it. Do not edit any file and do not post anything yourself; the
script posts what you output.";

/// Envoie la commande `/code-review` dans sa propre session.
///
/// Un prompt complet et rien d'autre : une revue n'a ni préambule ni portée,
/// à la différence de la boucle — chaque passe est un texte entier.
struct AskInline {
    level: String,
    spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<ReviewState> for AskInline {
    async fn run(&self, open: &mut Open<'_, ReviewState>) -> Outcome<Verdict> {
        let num = open.state.pr().num.clone();
        let prompt = format!("/code-review {} {num} --comment", self.level);
        ask_and_record(open, &prompt, "inline", 1, &num, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

/// Compose et envoie le prompt de la passe « brief ».
struct AskBrief {
    no_inline: bool,
    spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl SessionAction<ReviewState> for AskBrief {
    async fn run(&self, open: &mut Open<'_, ReviewState>) -> Outcome<Verdict> {
        let pr = open.state.pr().clone();
        let findings = gates::findings(open.ctx, self.no_inline);
        let prompt = splice(
            BRIEF_PROMPT,
            &[
                ("num", &pr.num),
                ("title", &pr.title),
                ("head", &pr.head),
                ("base", &pr.base),
                ("findings", &findings),
            ],
        );
        ask_and_record(open, &prompt, "brief", 1, &pr.num, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

/// Ce qu'une revue a besoin pour monter sa table.
pub struct Wiring {
    /// Le tableau de PR, et là où le commentaire se poste.
    pub gh: Rc<dyn GitHub>,
    /// Ce qui ouvre une session payante — ou la répète à blanc.
    pub sessions: Rc<dyn harness_core::adapters::agent::SessionFactory>,
    /// Où la dépense d'une passe est consignée.
    pub spending: Rc<dyn Spending>,
    /// Le dossier d'une revue — commentaire gardé, registre.
    pub review_dir: std::path::PathBuf,
    /// Le niveau de `/code-review` pour la passe « inline ».
    pub level: String,
    /// `--no-inline`.
    pub no_inline: bool,
    /// Modèle et effort de la passe « inline ».
    pub inline: SessionSpec,
    /// Modèle et effort de la passe « brief ».
    pub brief: SessionSpec,
    /// L'instant à afficher dans l'en-tête du commentaire posté.
    pub now: fn() -> String,
}

/// La passe ligne-à-ligne : `/code-review`, sautée sous `--no-inline`.
#[must_use]
pub fn inline(wiring: &Wiring) -> Stage<ReviewState> {
    Stage {
        name: "inline".to_string(),
        pre: Some(Gate {
            name: "inline requiert",
            checks: vec![Box::new(InlinePassIsOff {
                no_inline: wiring.no_inline,
            })],
        }),
        post: None,
        body: StageBody::Session {
            spec: wiring.inline.clone(),
            sessions: Rc::clone(&wiring.sessions),
            actions: vec![Box::new(AskInline {
                level: wiring.level.clone(),
                spending: Rc::clone(&wiring.spending),
            })],
        },
    }
}

/// La passe de synthèse : lit `inline`, écrit les notes pour l'humain.
#[must_use]
pub fn brief(wiring: &Wiring) -> Stage<ReviewState> {
    Stage {
        name: "brief".to_string(),
        pre: None,
        post: None,
        body: StageBody::Session {
            spec: wiring.brief.clone(),
            sessions: Rc::clone(&wiring.sessions),
            actions: vec![Box::new(AskBrief {
                no_inline: wiring.no_inline,
                spending: Rc::clone(&wiring.spending),
            })],
        },
    }
}

/// La publication : une étape locale, qui ne paie rien.
#[must_use]
pub fn publish(wiring: &Wiring) -> Stage<ReviewState> {
    Stage {
        name: "publish".to_string(),
        pre: Some(Gate {
            name: "publish requiert",
            checks: vec![Box::new(NothingIsPosted)],
        }),
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Publish {
                gh: Rc::clone(&wiring.gh),
                review_dir: wiring.review_dir.clone(),
                no_inline: wiring.no_inline,
                level: wiring.level.clone(),
                inline_model: wiring.inline.model.clone(),
                brief_model: wiring.brief.model.clone(),
                now: wiring.now,
            })],
        },
    }
}

/// Les deux passes et la publication, dans l'ordre.
#[must_use]
pub fn table(wiring: &Wiring) -> Vec<Stage<ReviewState>> {
    vec![inline(wiring), brief(wiring), publish(wiring)]
}

#[cfg(test)]
pub(crate) mod fake {
    //! Un câblage qui ne mène nulle part, pour monter une table sans réseau.

    use async_trait::async_trait;
    use harness_core::adapters::agent::{Session, SessionFactory, SessionSpec};
    use harness_core::adapters::store::spending::{Entry, Spending};
    use harness_core::domain::{Halt, Outcome};
    use std::path::PathBuf;
    use std::rc::Rc;

    use super::Wiring;
    use crate::common::fake_github::FakeGitHub;

    pub struct NoSessions;

    #[async_trait(?Send)]
    impl SessionFactory for NoSessions {
        async fn open(&self, _spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
            Err(Halt::Failed(
                "aucune session ne doit s'ouvrir dans ce test".to_string(),
            ))
        }
    }

    pub struct Nowhere;

    impl Spending for Nowhere {
        fn record(&self, _entry: &Entry<'_>) -> Outcome<()> {
            Err(Halt::Failed(
                "aucune dépense ne doit être consignée dans ce test".to_string(),
            ))
        }
    }

    pub fn wiring() -> Wiring {
        with(Rc::new(FakeGitHub::default()))
    }

    pub fn with(gh: Rc<FakeGitHub>) -> Wiring {
        let spec = SessionSpec {
            model: "sonnet".to_string(),
            effort: "medium".to_string(),
        };
        Wiring {
            gh,
            sessions: Rc::new(NoSessions),
            spending: Rc::new(Nowhere),
            review_dir: PathBuf::from("/tmp/review"),
            level: "medium".to_string(),
            no_inline: false,
            inline: spec.clone(),
            brief: spec,
            now: || "2026-10-02 16:00".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_inline_then_brief_then_publish() {
        let names: Vec<String> = table(&fake::wiring())
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(names, ["inline", "brief", "publish"]);
    }
}
