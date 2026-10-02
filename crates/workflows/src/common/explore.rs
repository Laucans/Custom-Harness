//! L'exploration : une lecture gratuite, une session qui condense, une carte.
//!
//! Ce que ça remplace : plusieurs sessions payées qui relisaient chacune
//! CLAUDE.md, `docs/ARCHITECTURE.md` et `docs/PROJECT.md` avant d'écrire
//! trois paragraphes.
//!
//! **Deux entrées, dont une seule paie :**
//!
//! 1. [`Ground`] colle les trois documents verbatim et la liste des fichiers
//!    suivis dans l'état. Aucune session, aucun jugement, aucune perte ;
//! 2. la session d'exploration lit ça, plus le code que le sujet touche, et
//!    rend une carte compacte — la seule des deux qui paie.
//!
//! **Dans `common/` parce que rien ici ne nomme un workflow** : un workflow
//! branche ces deux entrées en implémentant [`Explored`] sur son état. Seul
//! le raffinage s'en sert aujourd'hui ; ça resterait vrai le jour où un
//! second voudrait la même carte.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::adapters::agent::{SessionFactory, SessionSpec};
use harness_core::adapters::shell::disk::Disk;
use harness_core::adapters::shell::git::Repo;
use harness_core::adapters::store::spending::Spending;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{
    Action, Context, Open, SessionAction, Stage, StageBody, Verification, ask_and_record,
};
use harness_core::traces::Logbook;

/// Le nom de l'entrée gratuite, dans le journal et sur le registre.
pub const GROUND: &str = "ground";
/// Le nom de l'entrée payante.
pub const SKILL: &str = "explore";

/// Les documents collés verbatim dans le prompt de l'explorateur.
///
/// Verbatim et non résumés : ils pèsent une vingtaine de kilo-octets en
/// tout, et payer un modèle pour les condenser coûterait plus que les
/// quelques milliers de tokens de contexte que ça économise — en perdant la
/// formulation exacte des contraintes, qui est justement ce qui compte.
pub const GROUNDING: [&str; 3] = ["CLAUDE.md", "docs/ARCHITECTURE.md", "docs/PROJECT.md"];

/// Combien de fichiers suivis sont listés avant qu'on coupe.
pub const TREE_LINES: usize = 1500;

/// Ce que la carte a le droit de peser, en caractères (~4k tokens).
pub const BUDGET: usize = 16_000;

const ABSENT: &str = "(absent from this repository)";
const CUT: &str = "\n[... truncated: the map ran over its budget ...]";

/// Ce qu'une étape reçoit quand un dry-run n'a fait tourner personne.
///
/// Dit plutôt que laissé vide : les prompts qu'un dry-run écrit sur disque
/// servent à être relus, et un `{repo_context}` blanc s'y lirait comme une
/// injection cassée plutôt que comme une session non payée.
const DRY_RUN: &str = "--- REPO MAP (established once for this run) ---\n\
(dry run — no exploration session was paid, so there is no map)\n\
--- END REPO MAP ---";

const MAPPED: &str = "--- REPO MAP (established once for this run) ---
{digest}
--- END REPO MAP ---

A session read this repository for you and wrote the map above, so you do not
have to orient yourself: no directory listing, no search for where something
lives, no re-reading of CLAUDE.md or the docs. Open a file only to confirm an
exact detail the map does not carry — a signature, a field name, the precise
wording of a constraint — and only where your answer actually depends on it.
If the map and the code disagree, the code wins and you say so in what you
write.";

const UNMAPPED: &str = "--- REPO MAP (not established for this run) ---
No map was established, so ground what you write in the repository yourself
rather than guessing: CLAUDE.md carries the constraints that are not visible
in the code, docs/ARCHITECTURE.md the technical design, docs/PROJECT.md the
product, and the code the task touches carries the rest. Read whatever you
need. Naming a module that does not exist is worse than naming none.
--- END REPO MAP ---";

const EXPLORE_PROMPT: &str =
    "You are establishing the map of this repository that every later session of
this run will work from. You write it once; several paid sessions read it
instead of exploring on their own. Nothing else in this run will read the
repository from scratch, so what you leave out is what they will not know.

Here is what this run is working on:
<subject>
{subject}
</subject>

Below is the repository's own documentation, verbatim, plus the list of every
file git tracks. You do not need to open any of these — they are already
here. Read the **code** that the subject above actually touches, and only
that: the modules it names or implies, their neighbours, and the tests that
cover them.

<repository>
{brief}
</repository>

Write the map. It has to fit in about {budget} characters, so it is a map and
not a copy — every line that does not change what a later session would write
is a line that costs several sessions something and buys them nothing. Cover,
in this order and under these exact headings:

### Constraints
The rules from CLAUDE.md that bear on this subject, stated as rules. Quote a
constraint verbatim where its precise wording is what matters; summarise the
rest. Leave out what this subject cannot touch.

### Where things live
The modules the subject touches, by real path, and what each one is for in a
sentence. Name the layer boundaries that apply and what they forbid. This is
the section that makes a later session able to name a file without looking.

### Signatures and shapes
The functions, classes, types and fields a later session would have to name
to write a plan: their real names, their arguments, what they return. Exact
spelling matters more than completeness here — a name that is almost right
sends someone looking for it.

### Commands
The commands this repository actually runs to build, test, lint and verify,
copied from where they are documented rather than guessed.

### What is already true
What exists today that the subject assumes does not, or assumes differently:
a module already there, a convention already in force, a decision already
made. Be specific, and say where you saw it. This section is the one that
stops a later session from planning work that is already done.

Output the map and nothing else — no preamble, no closing remark, no code
fence around the whole answer. Use the four `### ` headings above, exactly as
spelled. Never write a level-2 heading (`## `): what you write is spliced
into prompts whose own structure uses them.

Change no file, run no command that writes, post no comment, touch no issue
and no label. You are reading.

The issues of this repository are public and what you write here flows into
them. Never write the value of a secret, a token, a key, a password, or a URL
that carries one — name the variable and say where it lives.";

/// Ce qu'un état doit porter pour que ces deux entrées tournent.
///
/// `subject` dit ce que ce run travaille — c'est ce qui décide quelles
/// parties du dépôt comptent. `brief` est où l'étape gratuite dépose sa
/// lecture, et seul l'explorateur le lit : il ne va jamais aux étapes qui
/// suivent, qui ne reçoivent que la carte.
pub trait Explored {
    /// Où l'étape gratuite dépose sa lecture.
    fn brief_mut(&mut self) -> &mut String;
    /// Ce que ce run travaille.
    fn subject(&self) -> String;
    /// Le round courant — pour la colonne `round` du registre.
    fn round_no(&self) -> u32;
    /// La task facturée — pour la colonne `task` du registre.
    fn task(&self) -> String;
}

/// Les fichiers suivis, coupés au budget.
#[must_use]
pub fn tree(files: &[String]) -> String {
    if files.len() <= TREE_LINES {
        return files.join("\n");
    }
    format!(
        "{}\n[... truncated: only the first {TREE_LINES} tracked files are listed ...]",
        files[..TREE_LINES].join("\n")
    )
}

/// La carte, ramenée dans son budget. Le dit quand elle déborde.
///
/// Coupée plutôt que refusée : une carte trop longue reste une carte, et
/// faire échouer le run sur une session qui a bien travaillé coûterait plus
/// que les caractères en trop.
#[must_use]
pub fn fits(text: &str, log: Option<&Logbook>) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= BUDGET {
        return trimmed.to_string();
    }
    if let Some(log) = log {
        log.warn(&format!(
            "the repo map came back at {} characters for a {BUDGET} budget — \
             truncated; tighten common::explore if it keeps happening",
            trimmed.len()
        ));
    }
    format!("{}{CUT}", &trimmed[..BUDGET])
}

/// L'entrée gratuite : les documents du dépôt et son arbre, posés dans l'état.
pub struct Ground {
    /// De quoi lister les fichiers suivis.
    pub repo: Rc<dyn Repo>,
    /// De quoi lire les documents.
    pub disk: Rc<dyn Disk>,
    /// La racine du code.
    pub root: PathBuf,
}

#[async_trait(?Send)]
impl<S: Explored> Action<S> for Ground {
    async fn run(&self, ctx: &mut Context<S>) -> Outcome<Verdict> {
        let files = self.repo.tracked_files().await?;
        if files.is_empty() {
            return Err(Halt::Failed(
                "git listed no tracked file, so there is nothing to map — \
                 check that the workspace is a git repository"
                    .to_string(),
            ));
        }
        let mut blocks: Vec<String> = GROUNDING
            .iter()
            .map(|name| {
                let text = self
                    .disk
                    .read_to_string(&self.root.join(name))
                    .unwrap_or_else(|| ABSENT.to_string());
                format!("<file path=\"{name}\">\n{text}\n</file>")
            })
            .collect();
        blocks.push(format!(
            "<tracked-files count=\"{}\">\n{}\n</tracked-files>",
            files.len(),
            tree(&files)
        ));
        *ctx.state.brief_mut() = blocks.join("\n\n");
        Ok(Verdict::Continue)
    }
}

/// Le `skip` des deux entrées : `--explore` rend le dépôt à chaque étape.
pub struct TurnedOff {
    /// `--explore`.
    pub explore: bool,
}

#[async_trait(?Send)]
impl<S> Verification<S> for TurnedOff {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        if !self.explore {
            return Ok(Verdict::Continue);
        }
        Ok(Verdict::Skip(
            "--explore — no repo map; every stage reads the repository for \
             itself"
                .to_string(),
        ))
    }
}

/// L'entrée payante : envoie le brief, garde la carte dans `ctx.results`.
struct AskExplore {
    spending: Rc<dyn Spending>,
}

#[async_trait(?Send)]
impl<S: Explored> SessionAction<S> for AskExplore {
    async fn run(&self, open: &mut Open<'_, S>) -> Outcome<Verdict> {
        let subject = open.state.subject();
        let brief = open.state.brief_mut().clone();
        let round = open.state.round_no();
        let task = open.state.task();
        let prompt = splice(
            EXPLORE_PROMPT,
            &[
                ("subject", &subject),
                ("brief", &brief),
                ("budget", &grouped(BUDGET)),
            ],
        );
        ask_and_record(open, &prompt, SKILL, round, &task, self.spending.as_ref()).await?;
        Ok(Verdict::Continue)
    }
}

/// L'`after` de l'exploration : ce qu'elle doit avoir obtenu, et où il va.
///
/// Écrit un fichier — une mise en cache, pas un état partagé que la suite du
/// run lit : la carte de ce run-ci vit déjà dans `ctx.results`, et cette
/// écriture ne sert qu'à une reprise. C'est pourquoi elle reste une
/// `Verification` plutôt qu'une `Action` scindée à part : rien d'observable
/// par les étapes suivantes n'en dépend.
pub struct Keep {
    /// Où la carte de ce round est gardée.
    pub map_file: PathBuf,
}

#[async_trait(?Send)]
impl<S: Explored> Verification<S> for Keep {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(got) = ctx.results.get(SKILL) else {
            // `--explore` a fait sauter l'étape via sa pre-gate : rien à
            // garder.
            return Ok(Verdict::Continue);
        };
        let said = fits(&got.text, Some(&ctx.traces));
        if said.is_empty() {
            return Err(Halt::Failed(
                "the exploration came back empty, so every section would \
                 work without a map — re-run, or use --explore to give each \
                 section the repository back"
                    .to_string(),
            ));
        }
        if let Some(parent) = self.map_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(broke) = std::fs::write(&self.map_file, &said) {
            ctx.traces.warn(&format!(
                "the repo map could not be kept on disk ({broke}) — this \
                 round is fine, a resumed one would lose it"
            ));
        }
        ctx.traces.say(&format!(
            "repo map established — {} characters served to every stage of \
             this round",
            said.len()
        ));
        Ok(Verdict::Continue)
    }
}

/// Où la carte d'un round est gardée, à côté des autres artefacts.
#[must_use]
pub fn map_file_path(artifacts_dir: &Path, tag: &str) -> PathBuf {
    artifacts_dir.join(format!("{tag}-{SKILL}.map.md"))
}

/// Ce qu'une étape reçoit comme bloc `{repo_context}` : la carte, ou son
/// absence.
///
/// Une seule porte pour toutes les façons de n'avoir pas de carte —
/// `--explore`, un dry-run, un workflow qui ne branche pas ces deux entrées.
/// L'ordre compte : `--explore` passe avant tout le reste, sinon un run lancé
/// pour rendre le dépôt aux étapes leur servirait la carte qu'un run
/// précédent a laissée là.
#[must_use]
pub fn repo_context<S>(ctx: &Context<S>, explore: bool, map_file: &Path) -> String {
    if explore {
        return UNMAPPED.to_string();
    }
    if ctx.settings.dry_run {
        return DRY_RUN.to_string();
    }
    if let Some(got) = ctx.results.get(SKILL) {
        return block(&fits(&got.text, None));
    }
    // L'étape a été sautée sans être un dry-run — latent aujourd'hui, parce
    // que rien ici ne tourne `--stages` ; une reprise le retrouverait sur
    // disque plutôt que de perdre la carte.
    match std::fs::read_to_string(map_file) {
        Ok(said) if !said.trim().is_empty() => block(said.trim()),
        _ => UNMAPPED.to_string(),
    }
}

fn block(digest: &str) -> String {
    splice(MAPPED, &[("digest", digest)])
}

/// `16000` → `"16,000"` — Rust n'a pas de groupeur de milliers natif.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

/// Ce qu'il faut pour monter les deux entrées.
pub struct Wiring {
    /// De quoi lister les fichiers suivis.
    pub repo: Rc<dyn Repo>,
    /// De quoi lire les documents du dépôt.
    pub disk: Rc<dyn Disk>,
    /// La racine du code.
    pub root: PathBuf,
    /// Ce qui ouvre la session d'exploration.
    pub sessions: Rc<dyn SessionFactory>,
    /// Où sa dépense est consignée.
    pub spending: Rc<dyn Spending>,
    /// `--explore`.
    pub explore: bool,
    /// Modèle et effort de la session d'exploration.
    pub spec: SessionSpec,
    /// Où la carte de ce round tombe.
    pub map_file: PathBuf,
}

/// Les deux entrées, dans l'ordre, prêtes à être mises en tête d'une table.
#[must_use]
pub fn entries<S: Explored + 'static>(wiring: &Wiring) -> [Stage<S>; 2] {
    let ground = Stage {
        name: GROUND.to_string(),
        pre: Some(harness_core::execution::Gate {
            name: "ground requiert",
            checks: vec![Box::new(TurnedOff {
                explore: wiring.explore,
            })],
        }),
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Ground {
                repo: Rc::clone(&wiring.repo),
                disk: Rc::clone(&wiring.disk),
                root: wiring.root.clone(),
            })],
        },
    };
    let explore = Stage {
        name: SKILL.to_string(),
        pre: Some(harness_core::execution::Gate {
            name: "explore requiert",
            checks: vec![Box::new(TurnedOff {
                explore: wiring.explore,
            })],
        }),
        post: Some(harness_core::execution::Gate {
            name: "explore doit obtenir",
            checks: vec![Box::new(Keep {
                map_file: wiring.map_file.clone(),
            })],
        }),
        body: StageBody::Session {
            spec: wiring.spec.clone(),
            sessions: Rc::clone(&wiring.sessions),
            actions: vec![Box::new(AskExplore {
                spending: Rc::clone(&wiring.spending),
            })],
        },
    };
    [ground, explore]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_tree_is_not_cut() {
        let files = vec!["a.rs".to_string(), "b.rs".to_string()];
        assert_eq!(tree(&files), "a.rs\nb.rs");
    }

    #[test]
    fn a_huge_tree_is_cut_at_the_budget_and_says_so() {
        let files: Vec<String> = (0..TREE_LINES + 10).map(|n| n.to_string()).collect();
        let said = tree(&files);
        assert!(said.contains("truncated"));
        assert_eq!(said.lines().count(), TREE_LINES + 1);
    }

    #[test]
    fn a_map_within_budget_is_untouched() {
        assert_eq!(fits("  courte  ", None), "courte");
    }

    #[test]
    fn an_oversized_map_is_cut_and_says_so() {
        let huge = "x".repeat(BUDGET + 500);
        let said = fits(&huge, None);
        assert!(said.ends_with(CUT));
        assert_eq!(said.len(), BUDGET + CUT.len());
    }

    #[test]
    fn repo_context_names_the_explore_flag_before_anything_else() {
        use harness_core::execution::{Context, Settings};
        use harness_core::traces::Logbook;
        let ctx: Context<()> = Context::new(
            Settings {
                dry_run: true,
                stages: String::new(),
            },
            (),
            Logbook::null(),
        );
        // --explore l'emporte même sur un dry-run.
        assert_eq!(
            repo_context(&ctx, true, Path::new("/tmp/jamais-lu")),
            UNMAPPED
        );
    }

    #[test]
    fn repo_context_under_dry_run_says_no_session_was_paid() {
        use harness_core::execution::{Context, Settings};
        use harness_core::traces::Logbook;
        let ctx: Context<()> = Context::new(
            Settings {
                dry_run: true,
                stages: String::new(),
            },
            (),
            Logbook::null(),
        );
        assert_eq!(
            repo_context(&ctx, false, Path::new("/tmp/jamais-lu")),
            DRY_RUN
        );
    }

    #[test]
    fn repo_context_reads_what_the_session_just_produced() {
        use harness_core::adapters::agent::Reply;
        use harness_core::domain::Spend;
        use harness_core::execution::{Context, Settings};
        use harness_core::traces::Logbook;
        let mut ctx: Context<()> = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            (),
            Logbook::null(),
        );
        ctx.results.insert(
            SKILL.to_string(),
            Reply {
                text: "### Constraints\n...".to_string(),
                stop_line: None,
                spend: Spend::default(),
            },
        );
        let said = repo_context(&ctx, false, Path::new("/tmp/jamais-lu"));
        assert!(said.contains("### Constraints"));
        assert!(said.starts_with("--- REPO MAP (established once for this run) ---"));
        assert!(said.contains("--- END REPO MAP ---"));
    }
}
