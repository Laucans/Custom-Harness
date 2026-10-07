//! Exploration: a free read, a session that condenses, a map.
//!
//! What it replaces: several paid sessions that each re-read
//! CLAUDE.md, `docs/ARCHITECTURE.md`, and `docs/PROJECT.md` before writing
//! three paragraphs.
//!
//! **Two entries, only one of which charges:**
//!
//! 1. [`Ground`] glues the three documents verbatim and the list of tracked
//!    files into state. No session, no judgment, no loss.
//! 2. The exploration session reads that, plus the code the subject touches, and
//!    produces a compact map — the only one of the two that charges.
//!
//! **In `common/` because nothing here names a workflow**: a workflow
//! wires these two entries by implementing [`Explored`] on its state. Only
//! refinement uses it today; it would still be true the day a
//! second one wanted the same map.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use async_trait::async_trait;
use harness_core::domain::prompts::splice;
use harness_core::domain::{Halt, Outcome, Verdict};
use harness_core::execution::{
    Action, Context, Open, SessionAction, Stage, StageBody, Verification, ask_and_record,
};
use harness_core::ports::agent::{SessionFactory, SessionSpec};
use harness_core::ports::shell::disk::Disk;
use harness_core::ports::shell::git::Repo;
use harness_core::ports::store::spending::Spending;
use harness_core::traces::Logbook;

/// Name of the free entry, in logs and registry.
pub const GROUND: &str = "ground";
/// Name of the paid entry.
pub const SKILL: &str = "explore";

/// Documents pasted verbatim into the explorer's prompt.
///
/// Verbatim, not summarized: they weigh about twenty kilobytes total,
/// and paying a model to condense them would cost more than the
/// few thousand context tokens it would save — while losing the
/// exact wording of constraints, which is what matters.
pub const GROUNDING: [&str; 3] = ["CLAUDE.md", "docs/ARCHITECTURE.md", "docs/PROJECT.md"];

/// How many tracked files are listed before truncation.
pub const TREE_LINES: usize = 1500;

/// How much the map is allowed to weigh, in characters (~4k tokens).
pub const BUDGET: usize = 16_000;

const ABSENT: &str = "(absent from this repository)";
const CUT: &str = "\n[... truncated: the map ran over its budget ...]";

/// What a stage receives when a dry-run ran no one.
///
/// Stated rather than left empty: the prompts a dry-run writes to disk
/// are meant to be re-read, and a blank `{repo_context}` would read as a
/// broken injection rather than an unpaid session.
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

/// What state must carry for these two entries to run.
///
/// `subject` says what this run works on — it decides which
/// parts of the repo matter. `brief` is where the free stage deposits its
/// reading, and only the explorer reads it: it never goes to the stages that
/// follow, which only receive the map.
pub trait Explored {
    /// Where the free stage deposits its reading.
    fn brief_mut(&mut self) -> &mut String;
    /// What this run works on.
    fn subject(&self) -> String;
    /// The current round — for the `round` column in the registry.
    fn round_no(&self) -> u32;
    /// The charged task — for the `task` column in the registry.
    fn task(&self) -> String;
}

/// Tracked files, cut to budget.
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

/// The map, brought within budget. States when it overflows.
///
/// Truncated rather than rejected: an oversized map is still a map, and
/// failing the run over a session that worked well would cost more
/// than the excess characters.
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
    // At a char boundary: slicing a `&str` mid-character panics, and the map is
    // prose a session wrote — on this repository, in French. An accent landing
    // on the budget would take down a run that had already paid for the map.
    let mut cut = BUDGET;
    while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{CUT}", &trimmed[..cut])
}

/// The free entry: the repo's documents and its tree, placed in state.
pub struct Ground {
    /// For listing tracked files.
    pub repo: Rc<dyn Repo>,
    /// For reading documents.
    pub disk: Rc<dyn Disk>,
    /// The code root.
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

/// The skip for both entries: `--explore` gives the repo to each stage.
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

/// The paid entry: sends the brief, keeps the map in `ctx.results`.
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

/// The `after` of exploration: what it must have obtained, and where it goes.
///
/// Writes a file — a cache, not shared state that the rest of the
/// run reads: this run's map already lives in `ctx.results`, and this
/// write only serves a resume. That is why it stays a
/// `Verification` rather than a separate split `Action`: nothing observable
/// by later stages depends on it.
///
/// Records the commit beside the map, which is what lets
/// [`AlreadyMapped`] skip the next exploration without guessing whether the
/// digest is still true.
pub struct Keep {
    /// The artifacts folder for this target (an issue, a round…).
    pub artifacts_dir: PathBuf,
    /// What answers which commit the map was drawn at.
    pub repo: Rc<dyn Repo>,
}

#[async_trait(?Send)]
impl<S: Explored> Verification<S> for Keep {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        if ctx.settings.dry_run {
            return Ok(Verdict::Continue);
        }
        let Some(got) = ctx.results.get(SKILL) else {
            // `--explore` skipped the stage via its pre-gate: nothing to
            // keep.
            return Ok(Verdict::Continue);
        };
        let map_file = map_file_path(&self.artifacts_dir);
        let said = fits(&got.text, Some(&ctx.traces));
        if said.is_empty() {
            return Err(Halt::Failed(
                "the exploration came back empty, so every section would \
                 work without a map — re-run, or use --explore to give each \
                 section the repository back"
                    .to_string(),
            ));
        }
        if let Some(parent) = map_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(broke) = std::fs::write(&map_file, &said) {
            ctx.traces.warn(&format!(
                "the repo map could not be kept on disk ({broke}) — this \
                 round is fine, a resumed one would pay to redraw it"
            ));
        }
        // The commit last, and only if the map landed: a recorded commit with
        // no map beside it would claim a cache that does not exist.
        if map_file.exists() {
            match self.repo.head_sha().await {
                Ok(head) => {
                    let _ = std::fs::write(sha_file_path(&self.artifacts_dir), head.trim());
                }
                Err(broke) => ctx.traces.warn(&format!(
                    "the map was kept but the commit it was drawn at could not \
                     be read ({broke}) — the next round will redraw it"
                )),
            }
        }
        ctx.traces.say(&format!(
            "repo map established — {} characters served to every stage of \
             this round",
            said.len()
        ));
        Ok(Verdict::Continue)
    }
}

/// Where the map is kept, alongside the target's other artifacts.
///
/// **One file per target, not one per round.** The repository does not change
/// between two rounds of the same issue, so a map per round meant paying to
/// redraw the same digest — on one roadmap item, thirteen explorations for
/// one repository. What the map depends on is the commit, and that is what
/// [`sha_file_path`] records beside it.
#[must_use]
pub fn map_file_path(artifacts_dir: &Path) -> PathBuf {
    artifacts_dir.join(format!("{SKILL}.map.md"))
}

/// Where the commit the map was drawn at is kept.
///
/// A sidecar rather than a header inside the map: the map is served verbatim
/// into prompts, and a line of our own bookkeeping has no business there.
#[must_use]
pub fn sha_file_path(artifacts_dir: &Path) -> PathBuf {
    artifacts_dir.join(format!("{SKILL}.map.sha"))
}

/// The commit a kept map was drawn at, or `None` when there is no usable map.
///
/// Both files are required: a map with no recorded commit cannot be known to
/// be current, and a recorded commit with no map serves nothing.
fn mapped_at(artifacts_dir: &Path) -> Option<String> {
    let map = std::fs::read_to_string(map_file_path(artifacts_dir)).ok()?;
    if map.trim().is_empty() {
        return None;
    }
    let sha = std::fs::read_to_string(sha_file_path(artifacts_dir)).ok()?;
    let sha = sha.trim();
    (!sha.is_empty()).then(|| sha.to_string())
}

/// The map on disk was drawn at the commit the checkout is on now.
///
/// Skips the paid exploration in that case: the digest is a function of the
/// tracked files and the grounding documents, so at an unchanged `HEAD` a
/// second exploration buys the same text. A moved `HEAD` re-explores, which
/// is what keeps the cache from going stale silently.
///
/// Dirty files are deliberately ignored: a session commits its work, and
/// treating an uncommitted edit as a new repository would re-explore on every
/// round of every run.
pub struct AlreadyMapped {
    /// What answers which commit the checkout is on.
    pub repo: Rc<dyn Repo>,
    /// The artifacts folder for this target.
    pub artifacts_dir: PathBuf,
}

#[async_trait(?Send)]
impl<S> Verification<S> for AlreadyMapped {
    async fn verify(&self, _ctx: &Context<S>) -> Outcome<Verdict> {
        let Some(drawn_at) = mapped_at(&self.artifacts_dir) else {
            return Ok(Verdict::Continue);
        };
        // A `HEAD` we cannot read is not a match: exploring again is the
        // answer that cannot be wrong.
        let Ok(head) = self.repo.head_sha().await else {
            return Ok(Verdict::Continue);
        };
        if head.trim() != drawn_at {
            return Ok(Verdict::Continue);
        }
        let short: String = drawn_at.chars().take(12).collect();
        Ok(Verdict::Skip(format!(
            "the repository is still at {short}, where the kept map was drawn \
             — serving it rather than paying to redraw it"
        )))
    }
}

/// What a stage receives as the `{repo_context}` block: the map, or its
/// absence.
///
/// One gate for all ways of having no map —
/// `--explore`, a dry-run, a workflow that does not wire these two entries.
/// Order matters: `--explore` takes precedence over everything else, else a run launched
/// to give the repo to stages would serve them a map from a
/// previous run left behind.
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
    // Stage was skipped without being a dry-run — latent today, because
    // nothing here runs `--stages`; a resume would find it on
    // disk rather than lose the map.
    match std::fs::read_to_string(map_file) {
        Ok(said) if !said.trim().is_empty() => block(said.trim()),
        _ => UNMAPPED.to_string(),
    }
}

fn block(digest: &str) -> String {
    splice(MAPPED, &[("digest", digest)])
}

/// `16000` → `"16,000"` — Rust has no native thousands grouper.
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

/// The ports of the two entries: read repo, open session, record.
///
/// Split from [`Config`] for the same reason as in workflows
/// (`ARCHITECTURE.md`): here what is injected, there what this run is worth.
pub struct Ports {
    /// For listing tracked files.
    pub repo: Rc<dyn Repo>,
    /// For reading the repo's documents.
    pub disk: Rc<dyn Disk>,
    /// What opens the exploration session.
    pub sessions: Rc<dyn SessionFactory>,
    /// Where its spending is recorded.
    pub spending: Rc<dyn Spending>,
}

/// Settings for the two entries.
pub struct Config {
    /// The code root.
    pub root: PathBuf,
    /// `--explore`: no map, each stage re-reads the repo.
    pub explore: bool,
    /// Model and effort for the exploration session.
    pub spec: SessionSpec,
    /// The artifacts folder for this target — the map and the commit it was
    /// drawn at land there.
    pub artifacts_dir: PathBuf,
}

/// The two entries, in order, ready to be put at the head of a table.
#[must_use]
pub fn entries<S: Explored + 'static>(ports: &Ports, config: &Config) -> [Stage<S>; 2] {
    let ground = Stage {
        name: GROUND.to_string(),
        pre: Some(harness_core::execution::Gate {
            name: "ground requires",
            checks: vec![Box::new(TurnedOff {
                explore: config.explore,
            })],
        }),
        post: None,
        body: StageBody::Local {
            actions: vec![Box::new(Ground {
                repo: Rc::clone(&ports.repo),
                disk: Rc::clone(&ports.disk),
                root: config.root.clone(),
            })],
        },
    };
    let explore = Stage {
        name: SKILL.to_string(),
        pre: Some(harness_core::execution::Gate {
            name: "explore requires",
            checks: vec![
                Box::new(TurnedOff {
                    explore: config.explore,
                }),
                // After `TurnedOff`: `--explore` means "no map at all", and
                // a kept map must not resurrect one.
                Box::new(AlreadyMapped {
                    repo: Rc::clone(&ports.repo),
                    artifacts_dir: config.artifacts_dir.clone(),
                }),
            ],
        }),
        post: Some(harness_core::execution::Gate {
            name: "explore must obtain",
            checks: vec![Box::new(Keep {
                artifacts_dir: config.artifacts_dir.clone(),
                repo: Rc::clone(&ports.repo),
            })],
        }),
        body: StageBody::Session {
            spec: config.spec.clone(),
            sessions: Rc::clone(&ports.sessions),
            actions: vec![Box::new(AskExplore {
                spending: Rc::clone(&ports.spending),
            })],
        },
    };
    [ground, explore]
}

#[cfg(test)]
pub(crate) mod fake {
    //! Ports that read a repo from a file, for tests of
    //! workflows that wire the map.
    //!
    //! The two reads **respond** rather than refuse: the free
    //! entry [`Ground`](super::Ground) runs even in dry-run — reading the
    //! repo costs nothing, and only a session pays. Everything else in
    //! `Repo` and `Disk` is unreachable from these two entries, and states so.

    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use async_trait::async_trait;
    use harness_core::adapters::agent::rehearsal::Rehearsal;
    use harness_core::domain::{Halt, Outcome};
    use harness_core::ports::agent::SessionSpec;
    use harness_core::ports::shell::disk::Disk;
    use harness_core::ports::shell::git::Repo;
    use harness_core::ports::shell::process::Ran;
    use harness_core::ports::store::spending::{Entry, Spending};
    use harness_core::traces::Logbook;

    use super::{Config, Ports};

    /// The commit [`OneFileRepo`] reports, so a test can write a map that
    /// matches it or one that does not.
    pub const HEAD: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c";

    /// A repo with one tracked file.
    pub struct OneFileRepo;

    #[async_trait(?Send)]
    impl Repo for OneFileRepo {
        async fn tracked_files(&self) -> Outcome<Vec<String>> {
            Ok(vec!["README.md".to_string()])
        }
        async fn current_branch(&self) -> Outcome<String> {
            unreachable!()
        }
        async fn head_sha(&self) -> Outcome<String> {
            // Reachable: the map cache is keyed on the commit.
            Ok(HEAD.to_string())
        }
        async fn dirty_files(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn has_branch(&self, _name: &str) -> Outcome<bool> {
            unreachable!()
        }
        async fn origin_has_branch(&self, _name: &str) -> Outcome<bool> {
            unreachable!()
        }
        async fn remote_url(&self, _remote: &str) -> Outcome<String> {
            unreachable!()
        }
        async fn default_branch(&self) -> Outcome<String> {
            unreachable!()
        }
        async fn local_branches(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn stashes(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn unpushed(&self) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn branches_at_risk(&self, _upstream: &str) -> Outcome<Vec<String>> {
            unreachable!()
        }
        async fn clone_repo(&self, _url: &str, _name: &str) -> Outcome<Ran> {
            unreachable!()
        }
        async fn fetch(&self) -> Outcome<Ran> {
            unreachable!()
        }
        async fn checkout(&self, _branch: &str, _force: bool) -> Outcome<Ran> {
            unreachable!()
        }
        async fn reset_hard(&self, _reference: &str) -> Outcome<Ran> {
            unreachable!()
        }
        async fn clean(&self) -> Outcome<Ran> {
            unreachable!()
        }
        async fn delete_branch(&self, _name: &str) -> Outcome<Ran> {
            unreachable!()
        }
        async fn create_local_branch(&self, _name: &str, _from: &str) -> Outcome<Ran> {
            unreachable!()
        }
        async fn stage_all(&self) -> Outcome<Ran> {
            unreachable!()
        }
        async fn commit(&self, _message: &str) -> Outcome<Ran> {
            unreachable!()
        }
        async fn push(&self, _branch: &str) -> Outcome<Ran> {
            unreachable!()
        }
    }

    /// A disk where none of the three documents exist — a normal response,
    /// not a failure.
    pub struct NoDocs;

    impl Disk for NoDocs {
        fn read_to_string(&self, _path: &Path) -> Option<String> {
            None
        }
        fn exists(&self, _path: &Path) -> bool {
            unreachable!()
        }
        fn create_dir_all(&self, _path: &Path) -> Outcome<()> {
            unreachable!()
        }
        fn remove_dir_all(&self, _path: &Path) -> Outcome<()> {
            unreachable!()
        }
        fn dir_names(&self, _path: &Path) -> Vec<String> {
            unreachable!()
        }
        fn write_to_string(&self, _path: &Path, _content: &str) -> Outcome<()> {
            unreachable!()
        }
    }

    /// A registry that refuses to write.
    pub struct Nowhere;

    impl Spending for Nowhere {
        fn record(&self, _entry: &Entry<'_>) -> Outcome<()> {
            Err(Halt::Failed(
                "no spending should be recorded in this test".to_string(),
            ))
        }
    }

    /// Test ports for the repo map.
    pub fn ports() -> Ports {
        Ports {
            repo: Rc::new(OneFileRepo),
            disk: Rc::new(NoDocs),
            sessions: Rc::new(Rehearsal::new(Logbook::null())),
            spending: Rc::new(Nowhere),
        }
    }

    /// Test config for the repo map, artifacts in this folder.
    pub fn config(artifacts_dir: PathBuf) -> Config {
        Config {
            root: PathBuf::new(),
            explore: false,
            spec: SessionSpec {
                model: "sonnet".to_string(),
                effort: "high".to_string(),
            },
            artifacts_dir,
        }
    }
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
    fn a_map_with_an_accent_on_the_budget_is_cut_without_panicking() {
        // The map is prose a session wrote, and on this repository it writes in
        // French. Slicing a `&str` mid-character panics, which would take down a
        // run that had already paid for the map.
        let straddling = format!("{}é{}", "a".repeat(BUDGET - 1), "b".repeat(100));
        assert!(
            !straddling.is_char_boundary(BUDGET),
            "the test must actually straddle the budget"
        );
        let said = fits(&straddling, None);
        assert!(said.contains("truncated"));
        assert!(!said.contains('é'), "the split character is dropped whole");
    }

    #[test]
    fn an_oversized_map_is_cut_and_says_so() {
        let huge = "x".repeat(BUDGET + 500);
        let said = fits(&huge, None);
        assert!(said.ends_with(CUT));
        assert_eq!(said.len(), BUDGET + CUT.len());
    }

    // --- the map cache -----------------------------------------------------

    fn anywhere(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("harness-map-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a test directory");
        dir
    }

    fn live() -> Context<()> {
        use harness_core::execution::Settings;
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            (),
            Logbook::null(),
        )
    }

    async fn verdict_for(dir: &Path) -> Verdict {
        AlreadyMapped {
            repo: Rc::new(fake::OneFileRepo),
            artifacts_dir: dir.to_path_buf(),
        }
        .verify(&live())
        .await
        .expect("a verdict")
    }

    #[tokio::test]
    async fn a_map_drawn_at_the_current_commit_is_served_not_redrawn() {
        // The saving this exists for: thirteen explorations of one repository
        // because the filename carried the round.
        let dir = anywhere("current");
        std::fs::write(map_file_path(&dir), "# the map").expect("a map");
        std::fs::write(sha_file_path(&dir), fake::HEAD).expect("a commit");
        let said = verdict_for(&dir).await;
        assert!(matches!(said, Verdict::Skip(_)), "{said:?}");
        let Verdict::Skip(why) = said else {
            unreachable!()
        };
        assert!(why.contains(&fake::HEAD[..12]), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_moved_commit_redraws_rather_than_serving_a_stale_map() {
        let dir = anywhere("moved");
        std::fs::write(map_file_path(&dir), "# the map").expect("a map");
        std::fs::write(sha_file_path(&dir), "aaaaaaaaaaaa").expect("a commit");
        assert_eq!(verdict_for(&dir).await, Verdict::Continue);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn no_map_at_all_explores() {
        let dir = anywhere("absent");
        assert_eq!(verdict_for(&dir).await, Verdict::Continue);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_map_whose_commit_was_not_recorded_cannot_be_trusted() {
        // It may predate any commit we know about; exploring again is the
        // answer that cannot be wrong.
        let dir = anywhere("unsigned");
        std::fs::write(map_file_path(&dir), "# the map").expect("a map");
        assert_eq!(verdict_for(&dir).await, Verdict::Continue);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_empty_map_beside_a_commit_serves_nothing() {
        let dir = anywhere("empty");
        std::fs::write(map_file_path(&dir), "   \n").expect("a map");
        std::fs::write(sha_file_path(&dir), fake::HEAD).expect("a commit");
        assert_eq!(verdict_for(&dir).await, Verdict::Continue);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_map_is_named_per_target_not_per_round() {
        // What made it a cache: two rounds of the same issue resolve to one
        // file, so the second serves what the first paid for.
        let dir = Path::new("/tmp/artifacts");
        assert_eq!(map_file_path(dir), dir.join("explore.map.md"));
        assert_ne!(map_file_path(dir), sha_file_path(dir));
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
        // --explore wins even over a dry-run.
        assert_eq!(
            repo_context(&ctx, true, Path::new("/tmp/never-read")),
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
        use harness_core::domain::Spend;
        use harness_core::execution::{Context, Settings};
        use harness_core::ports::agent::Reply;
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
        let said = repo_context(&ctx, false, Path::new("/tmp/never-read"));
        assert!(said.contains("### Constraints"));
        assert!(said.starts_with("--- REPO MAP (established once for this run) ---"));
        assert!(said.contains("--- END REPO MAP ---"));
    }
}
