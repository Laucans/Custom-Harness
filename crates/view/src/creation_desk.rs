//! A factory's creation at work: the record kept in the register, the AI's
//! questions and proposals, and the three long jobs — the factory founded
//! (its repository, its folder, `init-repo`), the proof of concept drawn as
//! mock-ups, the factory initialized (the design and the mock-ups published,
//! the roadmap task opened, the watch started).
//!
//! The founding runs in the view of the factory the human was on, the rest
//! in the view of the new one: it is a factory like any, so the design
//! system's easel and the mock-up's serve it as they serve every other.
//! One job at a time; each is a task, kept to be aborted on shutdown.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::data_desk::{DataDesk, Issue};
use crate::domain::annotations::Kind as PinKind;
use crate::domain::audit::{audit_of, audit_prompt, fix_prompt};
use crate::domain::creation::{
    self, ComponentRef, Context, Creation, DesignRef, Edit, Facts, PageRef, PinRef, RuleChange,
    RulesSent, Step, componentize_prompt, decompose_prompt, examples_part, examples_taken,
    grill_prompt, manifest_repair_prompt, mockups_prompt, not_componentized, parse_brief,
    parse_proposals, parse_requirements, parse_round, parse_ties, requirements_prompt, rule_prompt,
    rules_change_prompt, scope_prompt, unfenced,
};
use crate::domain::data_draft::{DataEdit, change_prompt, data_prompt, parse_data, rules_print};
use crate::domain::design::Entry;
use crate::domain::divergence;
use crate::domain::note_session::{About, mockup_rules};
use crate::kit_desk::{JobState, KitDesk};
use crate::ports::{Foundry, MockState, Mockups, NoteStore, Oneshot, Plant};
use crate::pulse::Pulse;
use harness_architecture::kit::{ComponentState, DESIGN_BRANCH, Design, DesignState, Example};
use harness_core::domain::Slug;
use harness_core::ports::store::factories::{Factories, id_from};
use harness_workflows::common::labels;

/// Why nothing asks the AI.
const NO_CLAUDE: &str = "no Claude at hand: the view found no `claude` to ask";

/// Why the mock-up cannot be drawn.
const NO_PRODUCT: &str = "this factory's repository cannot be read: no mock-up to draw";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the desk is made of.
pub struct Setup {
    /// The register of factories.
    pub register: Arc<dyn Factories + Send + Sync>,
    /// The factory this view is on.
    pub factory: String,
    /// What makes a new factory outside the view.
    pub foundry: Arc<dyn Foundry>,
    /// Where new factories' folders go.
    pub factories_dir: PathBuf,
    /// This factory's integration branch, and a new one's.
    pub integration_branch: String,
    /// The AI; `None` when no Claude is at hand.
    pub oneshot: Option<Arc<dyn Oneshot>>,
    /// The model the creation asks.
    pub model: String,
    /// The model that draws the mock-up and cuts it into micro-frontends:
    /// one session at work each.
    pub draw_model: String,
    /// The product's mock-up — `None` when the product cannot be read.
    pub mockups: Option<Arc<dyn Mockups>>,
    /// The design system's easel.
    pub kit: Option<KitDesk>,
    /// Where the roadmap task is opened.
    pub data: Option<DataDesk>,
    /// The watch the initialization starts.
    pub plant: Option<Arc<dyn Plant>>,
    /// The pins left on the mock-up: those no backlog issue carries go in
    /// the roadmap task.
    pub pins: Option<Arc<dyn NoteStore>>,
}

/// The design system and its kit, counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KitStatus {
    /// The design system is validated.
    pub validated: bool,
    /// Ticked components not generated yet, or generating.
    pub waiting: usize,
    /// Ticked components generated.
    pub generated: usize,
    /// Ticked components whose last generation failed.
    pub failed: usize,
    /// A generation is under way.
    pub running: bool,
    /// Nothing can generate: no Claude at hand.
    pub no_claude: bool,
}

/// Which long job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Work {
    /// A new factory founded: repository, folder, `init-repo`, register.
    Found,
    /// An existing repository connected as a factory.
    Connect,
    /// What the brief alone allows, drawn while the human answers the
    /// grilling: the design system's tokens (the kit follows).
    Prepare,
    /// The proof of concept drawn as mock-ups.
    Mockups,
    /// The accepted mock-up made again of micro-frontends.
    Componentize,
    /// The factory initialized.
    Initialize,
}

/// Where a job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Under way.
    Running,
    /// Done.
    Done,
    /// It stopped; `why` says why.
    Failed,
}

/// A long job, as the page follows it.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    /// Which.
    pub work: Work,
    /// Where it stands.
    pub state: State,
    /// What it did so far, one line a step.
    pub steps: Vec<String>,
    /// Why it failed; empty otherwise.
    pub why: String,
    /// The factory it made, once known.
    pub factory: String,
    /// What a step reported at length — `init-repo`'s report.
    pub report: String,
    /// When it started, in seconds since the epoch.
    pub started: u64,
    /// When it ended; 0 while it runs.
    pub ended: u64,
}

/// Now, in seconds since the epoch: when a job starts and ends.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A new factory, as the human asks for it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Founding {
    /// The repository's name, and the factory's.
    pub name: String,
    /// The account it is created under.
    pub owner: String,
    /// Private on GitHub.
    pub private: bool,
    /// The repository's description.
    pub description: String,
}

/// The creation desk.
#[derive(Clone)]
pub struct CreationDesk {
    setup: Arc<Setup>,
    job: Arc<Mutex<Option<Job>>>,
    /// The job's task: one at a time, so the last one is the only one alive.
    task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Held across every read-modify-write of the record.
    record: Arc<tokio::sync::Mutex<()>>,
    /// Beaten whenever the record is kept or the job moves: the page reads
    /// again.
    pulse: Pulse,
}

/// How request pins' change was settled on the mock-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled<'a> {
    /// Validated; `fork` is `(page, from, to)` when it was forked for a page.
    Kept {
        /// The fork made for a page: the page, the origin, the fork.
        fork: Option<(&'a str, &'a str, &'a str)>,
    },
    /// Taken back by an Undo.
    Undone,
}

impl CreationDesk {
    /// A desk with nothing under way.
    #[must_use]
    pub fn new(setup: Setup) -> Self {
        Self {
            setup: Arc::new(setup),
            job: Arc::default(),
            task: Arc::default(),
            record: Arc::default(),
            pulse: Pulse::default(),
        }
    }

    /// The same desk, its record and its jobs heard on `pulse`.
    #[must_use]
    pub fn with_pulse(mut self, pulse: Pulse) -> Self {
        self.pulse = pulse;
        self
    }

    /// The job under way or the last one.
    #[must_use]
    pub fn job(&self) -> Option<Job> {
        lock(&self.job).clone()
    }

    /// The integration branch this factory's work lands on.
    #[must_use]
    pub fn integration_branch(&self) -> &str {
        &self.setup.integration_branch
    }

    /// This factory's creation, when it is being created.
    ///
    /// # Errors
    ///
    /// The register cannot be read, or holds a record no longer readable.
    pub fn record(&self) -> Result<Option<Creation>, String> {
        self.setup
            .register
            .creation(&self.setup.factory)
            .map_err(|why| why.reason().to_string())?
            .map(|text| {
                serde_json::from_str::<Creation>(&text)
                    .map_err(|e| format!("the creation's record is not readable: {e}"))
            })
            .transpose()
    }

    fn keep(&self, record: &Creation) -> Result<(), String> {
        let text = serde_json::to_string(record).map_err(|e| e.to_string())?;
        let kept = self
            .setup
            .register
            .keep_creation(&self.setup.factory, &text)
            .map_err(|why| why.reason().to_string());
        self.pulse.beat();
        kept
    }

    fn existing(&self) -> Result<Creation, String> {
        self.record()?
            .ok_or_else(|| "this factory is not being created".to_string())
    }

    /// The design system and its kit, as the creation page shows them;
    /// `None` when nothing reads the product.
    pub async fn kit(&self) -> Option<KitStatus> {
        let reply = self.setup.kit.as_ref()?.reply().await.ok()?;
        let ticked: Vec<&Entry> = reply.catalog.iter().filter(|e| e.checked).collect();
        let count =
            |states: &[ComponentState]| ticked.iter().filter(|e| states.contains(&e.state)).count();
        Some(KitStatus {
            validated: reply.design.design.state == DesignState::Validated,
            waiting: count(&[ComponentState::Pending, ComponentState::Running]),
            generated: count(&[ComponentState::Generated]),
            failed: count(&[ComponentState::Failed]),
            running: reply
                .jobs
                .iter()
                .any(|j| matches!(j.state, JobState::Queued | JobState::Running)),
            no_claude: reply.no_claude,
        })
    }

    /// What the gates read outside the record: the design system validated,
    /// its kit generated.
    pub async fn facts(&self) -> Facts {
        Self::facts_of(self.kit().await)
    }

    /// The same, from a kit status read already.
    #[must_use]
    pub fn facts_of(kit: Option<KitStatus>) -> Facts {
        kit.map_or_else(Facts::default, |kit| Facts {
            design_validated: kit.validated,
            kit_waiting: kit.waiting,
        })
    }

    /// The human's edits, and the step they go to when they name one.
    ///
    /// # Errors
    ///
    /// No creation, the step's gate is closed, or the register refused.
    pub async fn save(&self, edit: Edit, step: Option<Step>) -> Result<Creation, String> {
        let facts = self.facts().await;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.edit(edit);
        if let Some(step) = step {
            record.go(step, facts)?;
        }
        self.keep(&record)?;
        Ok(record)
    }

    async fn ask(&self, prompt: String) -> Result<String, String> {
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let model = self.setup.model.clone();
        tokio::task::spawn_blocking(move || oneshot.ask(&model, &prompt))
            .await
            .map_err(|e| format!("the question did not finish: {e}"))?
    }

    /// The human's description kept, then sorted by the AI into the brief's
    /// parts — the brief written before is replaced.
    ///
    /// # Errors
    ///
    /// No creation, the brief's gate is closed, no description, no Claude,
    /// or an answer with no brief.
    pub async fn decompose(&self, description: String) -> Result<Creation, String> {
        if description.trim().is_empty() {
            return Err("describe the application first".to_string());
        }
        let facts = self.facts().await;
        let edit = |description: Option<String>, brief: Option<String>| Edit {
            description,
            brief,
            ..Edit::default()
        };
        {
            let _held = self.record.lock().await;
            let mut record = self.existing()?;
            record.may_enter(Step::Brief, facts)?;
            record.edit(edit(Some(description.clone()), None));
            self.keep(&record)?;
        }
        let brief = parse_brief(&self.ask(decompose_prompt(&description)).await?)?;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.edit(edit(None, Some(brief)));
        self.keep(&record)?;
        Ok(record)
    }

    /// The next grilling round, asked and added.
    ///
    /// # Errors
    ///
    /// No creation, no brief, no Claude, or an answer with no round.
    pub async fn grill(&self) -> Result<Creation, String> {
        let asked = self.existing()?;
        asked.may_enter(Step::Grilling, self.facts().await)?;
        let round = parse_round(&self.ask(grill_prompt(&asked)).await?)?;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.add_round(round);
        self.keep(&record)?;
        Ok(record)
    }

    /// The proof of concept's scope, proposed again.
    ///
    /// # Errors
    ///
    /// No creation, questions left open, no Claude.
    pub async fn propose_scope(&self) -> Result<Creation, String> {
        let asked = self.existing()?;
        asked.may_enter(Step::Scope, self.facts().await)?;
        let scope = unfenced(&self.ask(scope_prompt(&asked)).await?);
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.propose(scope);
        self.keep(&record)?;
        Ok(record)
    }

    /// What the brief allows before the grilling is over, as one job while
    /// the human answers: the design system's tokens drawn from the
    /// moodboard, the logo and the design brief when there is material and
    /// no tokens yet — the ticked kit components queue behind them. The
    /// scope is not drawn here: it waits for the grilling's decisions
    /// (`propose_scope`, on entering the scope step). Nothing here closes a
    /// gate.
    ///
    /// # Errors
    ///
    /// No creation, no brief, or another job under way.
    pub async fn prepare(&self) -> Result<(), String> {
        let record = self.existing()?;
        record.may_enter(Step::Grilling, self.facts().await)?;
        self.begin(Work::Prepare)?;
        let desk = self.clone();
        self.spawn(async move { desk.prepared().await });
        Ok(())
    }

    async fn prepared(&self) -> Result<(), String> {
        if let Some(kit) = &self.setup.kit {
            let reply = kit.reply().await?;
            let design = &reply.design.design;
            let material = !design.moodboard.is_empty()
                || !design.logo.is_empty()
                || !design.brief.trim().is_empty();
            let drawn = design.state != DesignState::None || design.candidate;
            if reply.no_claude {
                self.step("no Claude at hand: the design system waits for its tokens");
            } else if !material {
                self.step("no moodboard, logo or design brief yet: the tokens wait for them");
            } else if drawn {
                self.step("the design system has its tokens already");
                let queued = kit.start_generation(Vec::new()).await.unwrap_or(0);
                if queued > 0 {
                    self.step(&format!("{queued} kit component(s) queued"));
                }
            } else {
                self.step("drawing the design system's tokens from the moodboard, the logo and the design brief");
                let brief = design.brief.clone();
                kit.generate(brief).await?;
                let queued = kit.start_generation(Vec::new()).await.unwrap_or(0);
                self.step(&format!(
                    "the tokens are drawn; {queued} kit component(s) queued — look at the sheet, retouch, validate"
                ));
            }
        }
        Ok(())
    }

    /// The requirements, read by the AI off the scope and the mock-up as it
    /// stands — the human's own kept.
    ///
    /// # Errors
    ///
    /// No creation, no mock-up drawn, no Claude, or an answer with no
    /// requirements.
    pub async fn read_requirements(&self) -> Result<Creation, String> {
        self.existing()?
            .may_enter(Step::Rules, self.facts().await)?;
        self.requirements_read().await
    }

    /// The requirements read, with no gate: the drawing ahead reads them
    /// before the design system lets the human reach the review.
    async fn requirements_read(&self) -> Result<Creation, String> {
        let asked = self.existing()?;
        let ctx = self.context(&asked).await;
        let pages: Vec<String> = ctx.pages.iter().map(|p| p.path.clone()).collect();
        let components: Vec<String> = ctx.components.iter().map(|c| c.name.clone()).collect();
        let answer = self.ask(requirements_prompt(&asked, &ctx)).await?;
        let read = parse_requirements(&answer, &pages, &components)?;
        let ties = parse_ties(&answer, &pages, &components);
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.add_requirements(read);
        record.retie(&ties);
        self.keep(&record)?;
        Ok(record)
    }

    /// One rule read again by the AI off the scope and the mock-up as it
    /// stands; the others unchanged.
    ///
    /// # Errors
    ///
    /// No creation, no rule with that number, no Claude, or an answer with
    /// no rule.
    pub async fn reread_rule(&self, id: u32) -> Result<Creation, String> {
        let asked = self.existing()?;
        asked.may_enter(Step::Rules, self.facts().await)?;
        let rule = asked
            .requirements
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or_else(|| format!("no rule R{id}"))?;
        let ctx = self.context(&asked).await;
        let pages: Vec<String> = ctx.pages.iter().map(|p| p.path.clone()).collect();
        let components: Vec<String> = ctx.components.iter().map(|c| c.name.clone()).collect();
        let answer = self.ask(rule_prompt(&asked, &ctx, &rule)).await?;
        let read = parse_requirements(&answer, &pages, &components)?
            .into_iter()
            .next()
            .ok_or("the AI read the rule into nothing")?;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.reread(id, read)?;
        self.keep(&record)?;
        Ok(record)
    }

    /// The rules kept in step with a change of the mock-up: the AI proposes
    /// the rules it rewrites or adds, kept for the human to accept or
    /// refuse. No rule yet: nothing to keep in step, the record as it is.
    ///
    /// # Errors
    ///
    /// No creation, no Claude, or an answer with no proposals.
    pub async fn propose_rules(&self, change: RuleChange) -> Result<Creation, String> {
        let asked = self.existing()?;
        if asked.requirements.is_empty() {
            return Ok(asked);
        }
        let ctx = self.context(&asked).await;
        let pages: Vec<String> = ctx.pages.iter().map(|p| p.path.clone()).collect();
        let components: Vec<String> = ctx.components.iter().map(|c| c.name.clone()).collect();
        let answer = self.ask(rules_change_prompt(&asked, &ctx, &change)).await?;
        let read = parse_proposals(&answer, &pages, &components)?;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.propose_for(&change, read);
        self.keep(&record)?;
        Ok(record)
    }

    /// What request pins did to the rules, after their change was settled
    /// on the mock-up: validated (their proposals accepted, retied to a
    /// fork made for `page`), or taken back by their Undo. No creation:
    /// nothing to do. Answers what could not be done, in words.
    ///
    /// # Errors
    ///
    /// The record could not be kept.
    pub async fn settle_pins(
        &self,
        pins: &[String],
        settled: Settled<'_>,
    ) -> Result<Vec<String>, String> {
        let _held = self.record.lock().await;
        let Ok(mut record) = self.existing() else {
            return Ok(Vec::new());
        };
        let said = match settled {
            Settled::Kept { fork } => {
                if let Some((page, from, to)) = fork {
                    record.retie_fork(pins, page, from, to);
                }
                record.accept_pins(pins)
            }
            Settled::Undone => record.undo_pins(pins),
        };
        self.keep(&record)?;
        Ok(said)
    }

    /// The rules as the mock-up's history keeps them; `None`, no creation.
    pub async fn rules(&self) -> Option<crate::domain::creation::RulesState> {
        let _held = self.record.lock().await;
        self.existing().ok().map(|c| c.rules_state())
    }

    /// The rules as the mock-up's history kept them, written back.
    ///
    /// # Errors
    ///
    /// No creation, or the record could not be kept.
    pub async fn set_rules(
        &self,
        rules: crate::domain::creation::RulesState,
    ) -> Result<(), String> {
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.set_rules_state(rules);
        self.keep(&record)
    }

    /// One proposed change to the rules accepted or refused.
    ///
    /// # Errors
    ///
    /// No creation, no such proposal, or the rule it rewrites is gone.
    pub async fn settle_proposal(&self, key: u32, accept: bool) -> Result<Creation, String> {
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.settle_proposal(key, accept)?;
        self.keep(&record)?;
        Ok(record)
    }

    /// The architecture's guard asked about the cut as it stands: the
    /// checks read off it, then the AI's.
    ///
    /// # Errors
    ///
    /// No creation, its gate is closed, the mock-up is not cut yet, no
    /// Claude, or an answer that does not read.
    pub async fn audit(&self) -> Result<Creation, String> {
        let asked = self.existing()?;
        asked.may_enter(Step::Rules, self.facts().await)?;
        let from = asked
            .componentized
            .clone()
            .ok_or("the mock-up is not made of micro-frontends yet")?;
        let ctx = self.context(&asked).await;
        let answer = self
            .ask(audit_prompt(
                &ctx,
                "the language the micro-frontends are described in",
            ))
            .await?;
        let audit = audit_of(&answer, &ctx.components, &from)?;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.audit = Some(audit);
        self.keep(&record)?;
        Ok(record)
    }

    /// One finding taken away: its fix applied to the mock-up by a Claude
    /// Code session in the branch's workspace, or dismissed.
    ///
    /// # Errors
    ///
    /// No creation, no audit or no such finding, a blocking one dismissed,
    /// nothing can write, or the session failed.
    pub async fn resolve_finding(&self, id: u32, apply: bool) -> Result<Creation, String> {
        let asked = self.existing()?;
        let finding = asked
            .audit
            .as_ref()
            .and_then(|a| a.findings.iter().find(|f| f.id == id))
            .cloned()
            .ok_or_else(|| format!("no finding {id}"))?;
        if apply {
            let (Some(oneshot), Some(mockups)) =
                (self.setup.oneshot.clone(), self.setup.mockups.clone())
            else {
                return Err("nothing can write the mock-up here".to_string());
            };
            let branch = self.setup.integration_branch.clone();
            let rules = mockup_rules(&About {
                branch: branch.clone(),
                ..About::default()
            });
            let prompt = fix_prompt(&rules, &finding);
            let model = self.setup.model.clone();
            tokio::task::spawn_blocking(move || {
                let dir = mockups.workspace(&branch)?;
                oneshot.work_in(&model, &prompt, &dir)
            })
            .await
            .map_err(|e| format!("the session did not finish: {e}"))??;
        }
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record
            .audit
            .as_mut()
            .ok_or("no audit")?
            .resolve(id, apply)?;
        self.keep(&record)?;
        Ok(record)
    }

    /// The data model drafted by the AI off the rules, the cut and the
    /// mock-up's example data; a draft before it is replaced.
    ///
    /// # Errors
    ///
    /// No creation, the rules are not validated, no Claude, or an answer
    /// with no table.
    pub async fn draft_data(&self) -> Result<Creation, String> {
        let asked = self.existing()?;
        asked.may_enter(Step::Data, self.facts().await)?;
        let ctx = self.context(&asked).await;
        let fixtures = self.fixtures().await;
        let answer = self.ask(data_prompt(&asked, &ctx, &fixtures)).await?;
        let draft = parse_data(&answer, &rules_print(&asked))?;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.data = Some(draft);
        self.keep(&record)?;
        Ok(record)
    }

    /// The data model's structure changed by the AI as the human asks.
    ///
    /// # Errors
    ///
    /// No creation, no draft yet, no Claude, or an answer with no table.
    pub async fn change_data(&self, request: &str) -> Result<Creation, String> {
        let asked = self.existing()?;
        let draft = asked.data.as_ref().ok_or("no data model yet")?;
        if request.trim().is_empty() {
            return Err("say what to change".to_string());
        }
        let answer = self.ask(change_prompt(&asked, draft, request)?).await?;
        let changed = parse_data(&answer, &draft.from)?;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.data = Some(changed);
        self.keep(&record)?;
        Ok(record)
    }

    /// The human's retouches of the data model: words and policies.
    ///
    /// # Errors
    ///
    /// No creation, no draft yet, or the register refused.
    pub async fn edit_data(&self, edit: DataEdit) -> Result<Creation, String> {
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.data.as_mut().ok_or("no data model yet")?.edit(edit);
        self.keep(&record)?;
        Ok(record)
    }

    /// The data model validated: the four gates pass. Nothing is published.
    ///
    /// # Errors
    ///
    /// No creation, what keeps it, or the register refused.
    pub async fn validate_data(&self) -> Result<Creation, String> {
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.validate_data()?;
        self.keep(&record)?;
        Ok(record)
    }

    /// The example data the mock-up's micro-frontends read, a dozen files
    /// at most, each cut short: what the data model is drafted from.
    async fn fixtures(&self) -> Vec<(String, String)> {
        const FILES: usize = 12;
        const CHARS: usize = 1500;
        let Some(mockups) = self.setup.mockups.clone() else {
            return Vec::new();
        };
        let branch = self.setup.integration_branch.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(state) = mockups.state(&branch) else {
                return Vec::new();
            };
            let mut paths: Vec<String> = state
                .components
                .into_iter()
                .flat_map(|c| c.fixture)
                .collect();
            paths.sort();
            paths.dedup();
            paths
                .into_iter()
                .filter_map(|path| {
                    let bytes = mockups.file(&branch, &path).ok().flatten()?;
                    let text: String = String::from_utf8_lossy(&bytes)
                        .chars()
                        .take(CHARS)
                        .collect();
                    Some((path, text))
                })
                .take(FILES)
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    /// The rules validated: each has its criterion and is carried by the
    /// mock-up, none tied to what the mock-up has not. Nothing is published.
    ///
    /// # Errors
    ///
    /// No creation, what keeps the rules from it, or the register refused.
    pub async fn validate_rules(&self) -> Result<Creation, String> {
        // A tie to a page or a micro-frontend the mock-up has not is a gap
        // too, where there is a mock-up to read.
        let dangling = match &self.setup.mockups {
            Some(_) => {
                let ctx = self.context(&self.existing()?).await;
                let pages: Vec<String> = ctx.pages.iter().map(|p| p.path.clone()).collect();
                let components: Vec<String> =
                    ctx.components.iter().map(|c| c.name.clone()).collect();
                self.existing()?.dangling(&pages, &components)
            }
            None => Vec::new(),
        };
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        let gaps: Vec<String> = record.rule_gaps().into_iter().chain(dangling).collect();
        if !gaps.is_empty() {
            return Err(gaps.join("; "));
        }
        record.validate_rules()?;
        self.keep(&record)?;
        Ok(record)
    }

    /// Back to the brief, to draw the mock-ups again.
    ///
    /// # Errors
    ///
    /// No creation, or the register refused.
    pub async fn regenerate(&self) -> Result<Creation, String> {
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.regenerate();
        self.keep(&record)?;
        Ok(record)
    }

    /// The creation is over: the register forgets it.
    ///
    /// # Errors
    ///
    /// The factory is not initialized yet, or the register refused.
    pub async fn finish(&self) -> Result<(), String> {
        let _held = self.record.lock().await;
        let record = self.existing()?;
        if record.issue.is_empty() {
            return Err("initialize the factory first".to_string());
        }
        self.setup
            .register
            .end_creation(&self.setup.factory)
            .map_err(|why| why.reason().to_string())
    }

    /// The accounts a repository may be created under.
    ///
    /// # Errors
    ///
    /// `gh` could not say.
    pub async fn owners(&self) -> Result<Vec<String>, String> {
        let foundry = Arc::clone(&self.setup.foundry);
        tokio::task::spawn_blocking(move || foundry.owners())
            .await
            .map_err(|e| format!("the question did not finish: {e}"))?
    }

    /// Starts `work`, when nothing else is under way.
    fn begin(&self, work: Work) -> Result<(), String> {
        let busy = lock(&self.job)
            .as_ref()
            .is_some_and(|j| j.state == State::Running);
        if busy {
            return Err("another step of the creation is under way".to_string());
        }
        *lock(&self.job) = Some(Job {
            work,
            state: State::Running,
            steps: Vec::new(),
            why: String::new(),
            factory: String::new(),
            report: String::new(),
            started: now_secs(),
            ended: 0,
        });
        self.pulse.beat();
        Ok(())
    }

    fn update(&self, change: impl FnOnce(&mut Job)) {
        if let Some(job) = lock(&self.job).as_mut() {
            change(job);
        }
        self.pulse.beat();
    }

    fn step(&self, line: &str) {
        let line = line.to_string();
        self.update(|job| job.steps.push(line));
    }

    fn end(&self, outcome: Result<(), String>) {
        self.update(|job| {
            job.ended = now_secs();
            match outcome {
                Ok(()) => job.state = State::Done,
                Err(why) => {
                    job.state = State::Failed;
                    job.why = why;
                }
            }
        });
    }

    fn spawn(&self, work: impl std::future::Future<Output = Result<(), String>> + Send + 'static) {
        let desk = self.clone();
        let task = tokio::spawn(async move {
            let outcome = work.await;
            desk.end(outcome);
        });
        *lock(&self.task) = Some(task);
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Foundry) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let foundry = Arc::clone(&self.setup.foundry);
        tokio::task::spawn_blocking(move || f(foundry.as_ref()))
            .await
            .map_err(|e| format!("the step did not finish: {e}"))?
    }

    /// Where a new factory called `name` would live, and its id — refused
    /// when the register knows one there already.
    fn place_for(&self, name: &str) -> Result<(String, PathBuf), String> {
        let id = id_from(name);
        let root = self.setup.factories_dir.join(&id);
        let known = self
            .setup
            .register
            .list()
            .map_err(|why| why.reason().to_string())?;
        if let Some(f) = known.iter().find(|f| f.root == root) {
            return Err(format!("the factory `{}` lives there already", f.name));
        }
        Ok((id, root))
    }

    /// Founds a new factory: its repository on GitHub, its folder, the
    /// harness's `init-repo` there, its place in the register and the record
    /// of its creation. The page then opens it.
    ///
    /// # Errors
    ///
    /// A name GitHub would refuse, no owner, a folder taken, or another job
    /// under way. What fails later is the job's.
    pub fn found(&self, ask: Founding) -> Result<(), String> {
        let name = creation::repository_name(&ask.name)?;
        let owner = ask.owner.trim().to_string();
        if owner.is_empty() {
            return Err("choose the account the repository is created under".to_string());
        }
        let (_, root) = self.place_for(&name)?;
        self.begin(Work::Found)?;
        let desk = self.clone();
        let branch = self.setup.integration_branch.clone();
        self.spawn(async move {
            desk.step(&format!("creating the repository {owner}/{name} on GitHub"));
            let (o, n, d) = (owner.clone(), name.clone(), ask.description.clone());
            let slug = desk
                .blocking(move |f| f.create_repository(&o, &n, ask.private, &d))
                .await?;
            desk.settle(&name, &slug, root, &branch, true).await
        });
        Ok(())
    }

    /// Connects an existing repository as a factory: its folder, `init-repo`
    /// there, its place in the register. No creation steps: the page opens
    /// its plant.
    ///
    /// # Errors
    ///
    /// Not a GitHub repository, one a factory already drives, or another
    /// job under way.
    pub fn connect(&self, url: &str) -> Result<(), String> {
        let slug = Slug::parse(url.trim())
            .ok_or_else(|| format!("`{}` is not a GitHub repository", url.trim()))?;
        let known = self
            .setup
            .register
            .list()
            .map_err(|why| why.reason().to_string())?;
        if let Some(f) = known
            .iter()
            .find(|f| f.slug.eq_ignore_ascii_case(&slug.to_string()))
        {
            return Err(format!("the factory `{}` drives {slug} already", f.name));
        }
        let (_, root) = self.place_for(&slug.name)?;
        self.begin(Work::Connect)?;
        let desk = self.clone();
        let branch = self.setup.integration_branch.clone();
        self.spawn(async move {
            desk.settle(&slug.name, &slug.to_string(), root, &branch, false)
                .await
        });
        Ok(())
    }

    /// The rest of a founding, once the repository exists: the folder,
    /// `init-repo`, the register, and the creation's record when `creating`.
    async fn settle(
        &self,
        name: &str,
        slug: &str,
        root: PathBuf,
        branch: &str,
        creating: bool,
    ) -> Result<(), String> {
        self.step(&format!("making the factory's folder {}", root.display()));
        let at = root.clone();
        self.blocking(move |f| f.make_folder(&at)).await?;
        self.step(&format!("preparing {slug} (harness init-repo)"));
        let (at, s, b) = (root.clone(), slug.to_string(), branch.to_string());
        let initialized = self.blocking(move |f| f.init_repo(&at, &s, &b)).await?;
        let report = initialized.report.clone();
        self.update(|job| job.report = report);
        if !initialized.ready {
            self.step("init-repo left gaps a human must close: its report says which");
        }
        let factory = self
            .setup
            .register
            .enroll(&root, name, slug, branch)
            .map_err(|why| why.reason().to_string())?;
        if creating {
            let record =
                serde_json::to_string(&Creation::new(name, slug)).map_err(|e| e.to_string())?;
            self.setup
                .register
                .keep_creation(&factory.id, &record)
                .map_err(|why| why.reason().to_string())?;
        }
        self.step(&format!("the factory {} is in the register", factory.name));
        let id = factory.id;
        self.update(|job| job.factory = id);
        Ok(())
    }

    /// Draws the proof of concept as the product's mock-up, on the
    /// integration branch's workspace: one session at work, then the review.
    ///
    /// # Errors
    ///
    /// The step's gate is closed, nothing can draw, or another job is under
    /// way.
    pub async fn draw_mockups(&self) -> Result<(), String> {
        let record = self.existing()?;
        record.may_enter(Step::Mockups, self.facts().await)?;
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let mockups = self.setup.mockups.clone().ok_or(NO_PRODUCT)?;
        self.begin(Work::Mockups)?;
        let desk = self.clone();
        self.spawn(async move { desk.drawn(record, oneshot, mockups).await });
        Ok(())
    }

    /// Draws the proof of concept as soon as its scope is settled, while
    /// the human works on the design system: the mock-ups, their
    /// micro-frontends, the requirements tied to them — the mock-up step and
    /// the review find them ready. The tokens follow as variables; the kit's
    /// components are those generated by then. Drawn already: nothing runs.
    ///
    /// # Errors
    ///
    /// No scope, nothing can draw, or another job under way.
    pub async fn draw_ahead(&self) -> Result<(), String> {
        let record = self.existing()?;
        record.may_enter(Step::Design, self.facts().await)?;
        if record.mocked.is_some() {
            return Ok(());
        }
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let mockups = self.setup.mockups.clone().ok_or(NO_PRODUCT)?;
        self.begin(Work::Mockups)?;
        let desk = self.clone();
        self.spawn(async move { desk.drawn_ahead(record, oneshot, mockups).await });
        Ok(())
    }

    async fn drawn_ahead(
        &self,
        record: Creation,
        oneshot: Arc<dyn Oneshot>,
        mockups: Arc<dyn Mockups>,
    ) -> Result<(), String> {
        self.drawn(record, Arc::clone(&oneshot), Arc::clone(&mockups))
            .await?;
        // The job is named after what it does now: the page follows it.
        self.update(|job| job.work = Work::Componentize);
        let record = self.existing()?;
        self.componentize(&record, oneshot, mockups).await?;
        self.step("the AI reads the requirements off the scope and the mock-up");
        let read = self.requirements_read().await?;
        self.step(&format!("{} requirement(s) read", read.requirements.len()));
        Ok(())
    }

    async fn drawn(
        &self,
        record: Creation,
        oneshot: Arc<dyn Oneshot>,
        mockups: Arc<dyn Mockups>,
    ) -> Result<(), String> {
        let branch = self.setup.integration_branch.clone();
        let kit = match &self.setup.kit {
            Some(kit) => kit.index().await.map(|i| i.lines()).unwrap_or_default(),
            None => String::new(),
        };
        let rules = mockup_rules(&About {
            branch: branch.clone(),
            ..About::default()
        });
        let tokens_css = Self::design_tokens(&mockups).await;
        let mut prompt = mockups_prompt(&record, &rules, &kit, &tokens_css);
        let model = self.setup.draw_model.clone();
        let desk = self.clone();
        self.step("opening the integration branch's mock-up");
        let poll_mockups = mockups.clone();
        let poll_branch = branch.clone();
        let (mockups_after, branch_after) = (mockups.clone(), branch.clone());
        let (opened, at) = (mockups.clone(), branch.clone());
        let dir = tokio::task::spawn_blocking(move || {
            // A drawing starts from the branch as committed: one sent back
            // leaves nothing of the last behind.
            let _ = opened.discard(&at);
            opened.workspace(&at)
        })
        .await
        .map_err(|e| format!("the step did not finish: {e}"))??;
        prompt.push_str(&examples_part(&self.examples_lent(&mockups, &branch).await));
        let put_away = self.pins_put_away(&branch).await;
        if put_away > 0 {
            self.step(&format!(
                "{put_away} pin(s) of the last mock-up put away: a board's deleted list brings them back"
            ));
        }
        self.step("a Claude Code session draws the proof of concept's pages");
        // The workspace was just discarded above: nothing of a page is
        // there yet, so the poller starts from an empty `seen`.
        let poll = tokio::spawn(poll_mockup_pages(self.clone(), poll_mockups, poll_branch));
        let said = tokio::task::spawn_blocking(move || {
            let said = oneshot.work_in(&model, &prompt, &dir)?;
            let at = Session {
                oneshot: oneshot.as_ref(),
                model: &model,
                rules: &rules,
                dir: &dir,
            };
            read_back(&desk, &at, mockups.as_ref(), &branch)?;
            Ok::<_, String>(said)
        })
        .await
        .map_err(|e| format!("the session did not finish: {e}"));
        poll.abort();
        let said = said??;
        self.said_of_examples(&said);
        self.kept_on_design(&mockups_after, &branch_after, &tokens_css)
            .await;
        self.step("the mock-up is drawn");
        let facts = self.facts().await;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.drawn(said);
        // Drawn ahead, the human is still before the mock-up step: left there.
        if record.step >= Step::Mockups {
            record.go(Step::Rules, facts)?;
        }
        self.keep(&record)
    }

    /// The pins still on `branch`'s mock-up, put away (marked deleted, not
    /// destroyed) before a drawing replaces its pages: what they pointed at
    /// goes with them. Answers how many.
    async fn pins_put_away(&self, branch: &str) -> usize {
        let Some(pins) = self.setup.pins.clone() else {
            return 0;
        };
        let prefix = Self::boards_of(branch);
        let at = jiff::Timestamp::now().to_string();
        tokio::task::spawn_blocking(move || {
            let mut count = 0;
            for board in pins
                .boards()
                .unwrap_or_default()
                .into_iter()
                .filter(|b| b.starts_with(&prefix))
            {
                for kept in pins.notes(&board).unwrap_or_default() {
                    if kept.deleted_at.is_none()
                        && pins.mark_deleted(&board, &kept.note.id, Some(&at)).is_ok()
                    {
                        count += 1;
                    }
                }
            }
            count
        })
        .await
        .unwrap_or(0)
    }

    /// The interface examples of the design system a drawing of `branch`
    /// reads — the whole product's and its pages' — lent beside its pages
    /// under `design/examples/`, where `kept_on_design` takes them back.
    /// Answers the ones lent; none when the design system has none.
    async fn examples_lent(&self, mockups: &Arc<dyn Mockups>, branch: &str) -> Vec<Example> {
        let (mockups, branch) = (Arc::clone(mockups), branch.to_string());
        let lent = tokio::task::spawn_blocking(move || {
            let design: Design = mockups
                .file(DESIGN_BRANCH, crate::domain::design::DESIGN_JSON)?
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            let mut lent = Vec::new();
            for example in crate::domain::design::examples_for_pages(&design.examples) {
                let path = format!("design/{}", example.file);
                if let Some(bytes) = mockups.file(DESIGN_BRANCH, &path)? {
                    mockups.write(&branch, &path, &bytes)?;
                    lent.push(example);
                }
            }
            Ok::<_, String>(lent)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        match lent {
            Ok(lent) => {
                if !lent.is_empty() {
                    self.step(&format!(
                        "{} interface example(s) of the design system lent to the session",
                        lent.len()
                    ));
                }
                lent
            }
            Err(why) => {
                self.step(&format!("the interface examples could not be lent: {why}"));
                Vec::new()
            }
        }
    }

    /// What a session said it took from each interface example, one step each.
    fn said_of_examples(&self, said: &str) {
        for line in examples_taken(said) {
            self.step(&line);
        }
    }

    /// The design system's variables, as its own branch holds them: what a
    /// mock-up session builds on. Empty when there are none yet.
    async fn design_tokens(mockups: &Arc<dyn Mockups>) -> String {
        let mockups = Arc::clone(mockups);
        tokio::task::spawn_blocking(move || {
            mockups.file(
                harness_architecture::kit::DESIGN_BRANCH,
                crate::domain::design::TOKENS_CSS,
            )
        })
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
    }

    /// After a session on the mock-up: what it wrote under `design/` and
    /// `kit/` goes — the design system's own files are served there — and
    /// the variables it uses that the design system has not are said.
    async fn kept_on_design(&self, mockups: &Arc<dyn Mockups>, branch: &str, tokens_css: &str) {
        let tokens = creation::token_names(tokens_css);
        let (mockups, branch) = (Arc::clone(mockups), branch.to_string());
        let checked = tokio::task::spawn_blocking(move || {
            let files = mockups.files(&branch)?;
            let mut removed = Vec::new();
            let mut unknown = std::collections::BTreeSet::new();
            for f in files {
                if f.path.starts_with("design/") || f.path.starts_with("kit/") {
                    mockups.remove(&branch, &f.path)?;
                    // The examples lent are taken back, not reported.
                    if !f.path.starts_with("design/examples/") {
                        removed.push(f.path);
                    }
                } else if !tokens.is_empty()
                    && std::path::Path::new(&f.path).extension().is_some_and(|e| {
                        e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("css")
                    })
                {
                    let text = mockups.file(&branch, &f.path)?.unwrap_or_default();
                    unknown.extend(creation::unknown_tokens(
                        &String::from_utf8_lossy(&text),
                        &tokens,
                    ));
                }
            }
            Ok::<_, String>((removed, unknown))
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        match checked {
            Ok((removed, unknown)) => {
                if !removed.is_empty() {
                    self.step(&format!(
                        "{} file(s) the session wrote under design/ and kit/ removed: the design system's own are served ({})",
                        removed.len(),
                        removed.join(", ")
                    ));
                }
                if !unknown.is_empty() {
                    self.step(&format!(
                        "variables the design system has not, left to put right: {}",
                        unknown.into_iter().collect::<Vec<_>>().join(", ")
                    ));
                }
            }
            Err(why) => self.step(&format!("the design system could not be checked: {why}")),
        }
    }

    /// The proof of concept accepted: the factory is initialized — the
    /// design system and the mock-up published, the roadmap task opened, the
    /// watch started.
    ///
    /// # Errors
    ///
    /// No mock-up yet, or another job under way.
    pub async fn accept(&self) -> Result<Creation, String> {
        let facts = self.facts().await;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.accepted = true;
        record.go(Step::Initialize, facts)?;
        if !record.rules_validated {
            return Err("validate the rules first".to_string());
        }
        if !record.data_validated() {
            return Err("validate the data model first".to_string());
        }
        // The job first: an acceptance nothing starts is not kept.
        self.begin(Work::Initialize)?;
        self.keep(&record)?;
        let desk = self.clone();
        let kept = record.clone();
        self.spawn(async move { desk.initialized(kept).await });
        Ok(record)
    }

    /// The mock-up made again of micro-frontends, as its own job — before
    /// the review, so the requirements can be tied to them. Done already:
    /// nothing runs.
    ///
    /// # Errors
    ///
    /// No mock-up drawn, nothing can write, or another job under way.
    pub async fn componentize_now(&self) -> Result<(), String> {
        let record = self.existing()?;
        record.may_enter(Step::Rules, self.facts().await)?;
        if record.componentized.is_some() {
            return Ok(());
        }
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let mockups = self.setup.mockups.clone().ok_or(NO_PRODUCT)?;
        self.begin(Work::Componentize)?;
        let desk = self.clone();
        self.spawn(async move { desk.componentize(&record, oneshot, mockups).await });
        Ok(())
    }

    /// The boards of `branch`'s mock-up, as the easel names them.
    fn boards_of(branch: &str) -> String {
        let mut plain = String::new();
        for c in branch.chars() {
            if c.is_ascii_alphanumeric() {
                plain.push(c.to_ascii_lowercase());
            } else if !plain.ends_with('-') {
                plain.push('-');
            }
        }
        let plain = plain.trim_matches('-');
        format!(
            "mockup--{}--",
            if plain.is_empty() {
                "integration"
            } else {
                plain
            }
        )
    }

    /// What surrounds the record: the mock-up's pages and micro-frontends,
    /// the pins no backlog issue carries, the design system. Read as it
    /// stands; what cannot be read is left empty.
    async fn context(&self, record: &Creation) -> Context {
        let branch = self.setup.integration_branch.clone();
        let mut ctx = Context::default();
        if let Some(mockups) = self.setup.mockups.clone() {
            let at = branch.clone();
            if let Ok(Ok(state)) = tokio::task::spawn_blocking(move || mockups.state(&at)).await {
                ctx.pages = state
                    .pages
                    .iter()
                    .map(|p| PageRef {
                        title: p.title.clone(),
                        path: p.path.clone(),
                        objective: p.objective.clone(),
                    })
                    .collect();
                ctx.components = state
                    .components
                    .iter()
                    .map(|c| ComponentRef {
                        name: c.name.clone(),
                        system: c.system.clone(),
                        description: c.description.clone(),
                        needs: c.needs.clone(),
                        actions: c.actions.clone(),
                    })
                    .collect();
            }
        }
        if let Some(pins) = self.setup.pins.clone() {
            let prefix = Self::boards_of(&branch);
            let read = tokio::task::spawn_blocking(move || {
                let mut left = Vec::new();
                for board in pins
                    .boards()?
                    .into_iter()
                    .filter(|b| b.starts_with(&prefix))
                {
                    for kept in pins.notes(&board)? {
                        let note = kept.note;
                        let carried = kept.deleted_at.is_some()
                            || !note.issue.is_empty()
                            || note.text.trim().is_empty()
                            || matches!(note.kind, PinKind::Note | PinKind::Brainstorm);
                        if carried {
                            continue;
                        }
                        left.push(PinRef {
                            kind: note.kind.name().to_string(),
                            board: board.clone(),
                            target: note.target.clone(),
                            text: note.text.clone(),
                        });
                    }
                }
                Ok::<_, String>(left)
            })
            .await;
            if let Ok(Ok(left)) = read {
                ctx.pins = left;
            }
        }
        if let Some(kit) = &self.setup.kit
            && let Ok(reply) = kit.reply().await
        {
            ctx.design = DesignRef {
                validated_at: reply.design.design.validated_at.clone(),
                tokens: reply.design.tokens.len(),
                components: reply
                    .catalog
                    .iter()
                    .filter(|e| e.state == ComponentState::Generated)
                    .map(|e| e.name.clone())
                    .collect(),
            };
        }
        let _ = record;
        ctx
    }

    async fn initialized(&self, record: Creation) -> Result<(), String> {
        if record.componentized.is_none()
            && let (Some(oneshot), Some(mockups)) =
                (self.setup.oneshot.clone(), self.setup.mockups.clone())
        {
            self.componentize(&record, oneshot, mockups).await?;
        }
        let record = self.existing()?;
        let ctx = self.context(&record).await;
        // The mock-up is published first: `carry` always reads the
        // integration branch's latest head before splicing the design
        // system onto it, so it lands on top of the mock-up's commit — the
        // other way around, the mock-up's own push would meet a branch the
        // carry already moved, and be refused as not a fast-forward.
        self.publish_mockup(&record, &ctx).await?;
        self.publish_design(&record).await?;
        let record = self.existing()?;
        self.open_rules_task(&record).await?;
        let record = self.existing()?;
        if record.issue.is_empty() {
            let data = self
                .setup
                .data
                .clone()
                .ok_or("this factory's board cannot be reached: no roadmap task")?;
            self.step("opening the roadmap task");
            let (title, body) = creation::roadmap_issue(&record, &ctx);
            let url = data
                .open_issue(Issue {
                    title,
                    body,
                    labels: divergence::ISSUE_LABELS
                        .iter()
                        .map(|l| (*l).to_string())
                        .collect(),
                })
                .await?;
            self.step(&format!("the roadmap task: {url}"));
            let _held = self.record.lock().await;
            let mut kept = self.existing()?;
            kept.issue = url;
            self.keep(&kept)?;
        } else {
            self.step(&format!(
                "the roadmap task is open already: {}",
                record.issue
            ));
        }
        if let Some(plant) = self.setup.plant.clone() {
            self.step("starting the watch");
            let started = tokio::task::spawn_blocking(move || {
                if crate::domain::plant::running(plant.as_ref()).is_some() {
                    return Ok(None);
                }
                plant.start(None).map(Some)
            })
            .await
            .map_err(|e| format!("the step did not finish: {e}"))?;
            match started {
                Ok(Some(pid)) => self.step(&format!("the watch runs (pid {pid})")),
                Ok(None) => self.step("the watch was running already"),
                Err(why) => self.step(&format!(
                    "the watch did not start: {why} — start it from the plant"
                )),
            }
        }
        Ok(())
    }

    /// The rules' task, opened once, first of the roadmap: the rules and
    /// their tests come before any development.
    async fn open_rules_task(&self, record: &Creation) -> Result<(), String> {
        if !record.rules_issue.is_empty() {
            self.step(&format!(
                "the rules' task is open already: {}",
                record.rules_issue
            ));
            return Ok(());
        }
        let Some(data) = self.setup.data.clone() else {
            return Ok(());
        };
        self.step("opening the rules' task");
        let (title, body) = creation::rules_issue(record)?;
        let url = data
            .open_issue(Issue {
                title,
                body,
                labels: vec![labels::RULES.to_string(), labels::READY.to_string()],
            })
            .await?;
        self.step(&format!("the rules' task: {url}"));
        let _held = self.record.lock().await;
        let mut kept = self.existing()?;
        kept.rules_issue = url;
        kept.rules_sent.clone_from(&record.requirements);
        self.keep(&kept)
    }

    /// The rules changed since the factory received them, sent once they
    /// are validated again: a rules' task — every rule, the changed ones
    /// marked `conform` — and a conformity task, under the milestone the
    /// development works on so that its end waits on it, or a roadmap item
    /// when none is open.
    ///
    /// # Errors
    ///
    /// Not initialized, rules not validated, nothing changed, no board, or
    /// GitHub refused.
    pub async fn send_rules(&self) -> Result<Creation, String> {
        // Held throughout: a second click waits, and finds nothing to send.
        let _held = self.record.lock().await;
        let record = self.existing()?;
        if record.rules_issue.is_empty() {
            return Err("initialize the factory first".to_string());
        }
        if !record.rules_validated {
            return Err("validate the rules first".to_string());
        }
        let (title, body) = creation::rules_change_issue(&record)?;
        let data = self
            .setup
            .data
            .clone()
            .ok_or("this factory's board cannot be reached")?;
        let rules_url = data
            .open_issue(Issue {
                title,
                body,
                labels: vec![labels::RULES.to_string(), labels::READY.to_string()],
            })
            .await?;
        let (title, body) = creation::conformity_issue(&record, &rules_url);
        let conformity_url = match data.current_milestone().await? {
            Some(milestone) => {
                data.open_task(
                    milestone,
                    Issue {
                        title,
                        body,
                        labels: vec![labels::AGENT.to_string(), labels::READY.to_string()],
                    },
                )
                .await?
            }
            None => {
                data.open_issue(Issue {
                    title,
                    body,
                    labels: vec![labels::ROADMAP.to_string(), labels::READY.to_string()],
                })
                .await?
            }
        };
        let mut kept = self.existing()?;
        kept.rules_sent = record.requirements;
        kept.rules_changes.push(RulesSent {
            rules: rules_url,
            conformity: conformity_url,
        });
        self.keep(&kept)?;
        Ok(kept)
    }

    /// The record written in the repository beside the mock-up, then the
    /// mock-up published on the integration branch — or kept on a
    /// `mockup/*` branch when it refuses — and where it went kept.
    async fn publish_mockup(&self, record: &Creation, ctx: &Context) -> Result<(), String> {
        let Some(mockups) = self.setup.mockups.clone() else {
            return Ok(());
        };
        let branch = self.setup.integration_branch.clone();
        self.step(&format!(
            "writing the record of the creation (mock/{})",
            creation::RECORD_JSON
        ));
        let json = creation::record_json(record, ctx)?;
        let (_, md) = creation::roadmap_issue(record, ctx);
        let data = match &record.data {
            Some(d) => Some((
                d.model_json()?,
                serde_json::to_string_pretty(&d.aggregates).map_err(|e| e.to_string())?,
            )),
            None => None,
        };
        let at = branch.clone();
        let wrote = mockups.clone();
        tokio::task::spawn_blocking(move || {
            wrote.write(&at, creation::RECORD_JSON, json.as_bytes())?;
            if let Some((model, aggregates)) = data {
                wrote.write(&at, creation::DATA_MODEL, model.as_bytes())?;
                wrote.write(&at, creation::DATA_AGGREGATES, aggregates.as_bytes())?;
            }
            wrote.write(&at, creation::RECORD_MD, md.as_bytes())
        })
        .await
        .map_err(|e| format!("the step did not finish: {e}"))??;
        self.step("publishing the mock-up on the integration branch");
        let message = format!("mock: the proof of concept of {}", record.name);
        let (place, said) = tokio::task::spawn_blocking(move || {
            let state = mockups.state(&branch)?;
            if !state.edited {
                // Saved from the easel already: the code stage reads a `mockup/*` branch.
                // Nothing saved either: the branch carries it as committed.
                return Ok::<_, String>(if state.saved.is_empty() {
                    (branch, "nothing new to publish".to_string())
                } else {
                    let said = format!("saved on {} from the easel", state.saved);
                    (state.saved, said)
                });
            }
            Ok(match mockups.publish(&branch, &message) {
                Ok(commit) => {
                    let short = commit.get(..10).unwrap_or(&commit);
                    (format!("{branch}@{short}"), format!("published ({short})"))
                }
                // A protected branch: the code stage reads a `mockup/*` branch too.
                Err(why) => {
                    let saved = mockups.save(&branch, "proof-of-concept")?;
                    let said = format!("kept on {saved} ({why})");
                    (saved, said)
                }
            })
        })
        .await
        .map_err(|e| format!("the step did not finish: {e}"))??;
        self.step(&format!("the mock-up: {said}"));
        let _held = self.record.lock().await;
        let mut kept = self.existing()?;
        kept.mockup_at = place;
        self.keep(&kept)
    }

    /// The design system published on its branch and carried onto the
    /// integration branch; its commit and a refused carry kept.
    async fn publish_design(&self, record: &Creation) -> Result<(), String> {
        let Some(kit) = &self.setup.kit else {
            return Ok(());
        };
        self.step("publishing the design system");
        let (at, refused) = match kit
            .publish(format!("design: the design system of {}", record.name))
            .await
        {
            Ok(published) => {
                self.step(&creation::design_published(&published.refused));
                let short = published.commit.get(..10).unwrap_or(&published.commit);
                (short.to_string(), published.refused)
            }
            Err(why) => {
                self.step(&format!("the design system stays as it is: {why}"));
                (String::new(), String::new())
            }
        };
        let _held = self.record.lock().await;
        let mut kept = self.existing()?;
        kept.design_at = at;
        kept.design_refused = refused;
        self.keep(&kept)
    }

    /// Makes the accepted mock-up again of micro-frontends, in the
    /// integration branch's workspace, then checks every page is made of
    /// listed ones: what the micro-frontends' view and the code stage read.
    async fn componentize(
        &self,
        record: &Creation,
        oneshot: Arc<dyn Oneshot>,
        mockups: Arc<dyn Mockups>,
    ) -> Result<(), String> {
        let branch = self.setup.integration_branch.clone();
        let kit = match &self.setup.kit {
            Some(kit) => kit.index().await.map(|i| i.lines()).unwrap_or_default(),
            None => String::new(),
        };
        let rules = mockup_rules(&About {
            branch: branch.clone(),
            ..About::default()
        });
        let (mockups_after, branch_after) = (mockups.clone(), branch.clone());
        let tokens_css = Self::design_tokens(&mockups).await;
        let mut prompt = componentize_prompt(record, &rules, &kit, &tokens_css);
        prompt.push_str(&examples_part(&self.examples_lent(&mockups, &branch).await));
        let model = self.setup.draw_model.clone();
        self.step("a Claude Code session makes the mock-up again of micro-frontends");
        // Seeded with whatever is already there: a retry's own earlier
        // components are never reported as newly made.
        let seed_mockups = mockups.clone();
        let seed_branch = branch.clone();
        let seen: HashSet<String> = tokio::task::spawn_blocking(move || {
            seed_mockups
                .state(&seed_branch)
                .map(|state| state.components.into_iter().map(|c| c.name).collect())
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
        let poll = tokio::spawn(poll_componentize(
            self.clone(),
            mockups.clone(),
            branch.clone(),
            seen,
        ));
        let desk = self.clone();
        let said = tokio::task::spawn_blocking(move || {
            let dir = mockups.workspace(&branch)?;
            let said = oneshot.work_in(&model, &prompt, &dir)?;
            let at = Session {
                oneshot: oneshot.as_ref(),
                model: &model,
                rules: &rules,
                dir: &dir,
            };
            let state = read_back(&desk, &at, mockups.as_ref(), &branch)?;
            let mut pages = Vec::new();
            for page in &state.pages {
                let html = mockups.file(&branch, &page.path)?.unwrap_or_default();
                pages.push((
                    page.path.clone(),
                    String::from_utf8_lossy(&html).into_owned(),
                ));
            }
            let mut components = Vec::new();
            for c in &state.components {
                // An empty path is no file: the port reads nothing there.
                let there = mockups.file(&branch, &c.fragment)?.is_some();
                components.push((c.name.clone(), there));
            }
            Ok::<_, String>((said, not_componentized(&pages, &components)))
        })
        .await
        .map_err(|e| format!("the session did not finish: {e}"));
        poll.abort();
        let (said, problems) = said??;
        self.said_of_examples(&said);
        // Before the verdict: a cut refused leaves no example lent behind.
        self.kept_on_design(&mockups_after, &branch_after, &tokens_css)
            .await;
        if !problems.is_empty() {
            return Err(format!(
                "the mock-up is not made of micro-frontends yet: {}",
                problems.join("; ")
            ));
        }
        self.step("every page is made of micro-frontends");
        let _held = self.record.lock().await;
        let mut kept = self.existing()?;
        kept.componentized = Some(said);
        self.keep(&kept)
    }

    /// Stops what is under way.
    pub fn shutdown(&self) {
        let task = lock(&self.task).take();
        if let Some(task) = task {
            task.abort();
        }
    }
}

/// How many times a session is asked to repair a `mock/pages.json` the
/// view cannot read before the job stops on it.
const MANIFEST_REPAIRS: u32 = 2;

/// The session a job put to work, as a repair asks it again.
struct Session<'a> {
    oneshot: &'a dyn Oneshot,
    model: &'a str,
    rules: &'a str,
    dir: &'a std::path::Path,
}

/// `branch`'s mock-up read back after a session: a `mock/pages.json` that
/// does not read goes back to a session to repair, `MANIFEST_REPAIRS` times
/// at most — a job never ends done on a mock-up the view cannot show.
fn read_back(
    desk: &CreationDesk,
    session: &Session<'_>,
    mockups: &dyn Mockups,
    branch: &str,
) -> Result<MockState, String> {
    let mut repairs = 0;
    loop {
        match mockups.state(branch) {
            Ok(state) => return Ok(state),
            Err(why) if repairs < MANIFEST_REPAIRS => {
                repairs += 1;
                desk.step(&format!("{why} — a session repairs it"));
                session.oneshot.work_in(
                    session.model,
                    &manifest_repair_prompt(session.rules, &why),
                    session.dir,
                )?;
            }
            Err(why) => return Err(why),
        }
    }
}

/// How often the mock-up's drawing and the micro-frontends' remake are
/// polled for a page or a component that just landed.
const DRAW_POLL: Duration = Duration::from_secs(2);

/// Reports each page of `branch`'s mock-up as it gets a real fragment,
/// starting from nothing: `drawn()` always calls this on a freshly
/// discarded workspace, so no page is there yet to seed `seen` with. Runs
/// until aborted — `drawn()` stops it once `work_in` returns, success or
/// not; a read that fails is skipped, never reported as a reason the job
/// failed, since the real outcome is `work_in`'s alone.
async fn poll_mockup_pages(desk: CreationDesk, mockups: Arc<dyn Mockups>, branch: String) {
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        tokio::time::sleep(DRAW_POLL).await;
        let (mockups, at) = (mockups.clone(), branch.clone());
        let Ok(Ok(state)) = tokio::task::spawn_blocking(move || mockups.state(&at)).await else {
            continue;
        };
        let total = state.pages.len();
        for page in &state.pages {
            if page.hash.is_empty() || seen.contains(&page.path) {
                continue;
            }
            seen.insert(page.path.clone());
            desk.step(&format!(
                "drew the mock-up's \"{}\" page ({} of {total})",
                page.title,
                seen.len()
            ));
        }
    }
}

/// Reports each micro-frontend of `branch`'s mock-up as it gets a real
/// fragment, `seen` seeded with whatever `componentize()` found already
/// there before it started — a retry's own earlier components are not
/// reported as newly made. Runs until aborted, the same as above.
async fn poll_componentize(
    desk: CreationDesk,
    mockups: Arc<dyn Mockups>,
    branch: String,
    mut seen: HashSet<String>,
) {
    loop {
        tokio::time::sleep(DRAW_POLL).await;
        let (mockups, at) = (mockups.clone(), branch.clone());
        let Ok(Ok(state)) = tokio::task::spawn_blocking(move || {
            let state = mockups.state(&at)?;
            let mut built = Vec::new();
            for c in &state.components {
                built.push((c.name.clone(), mockups.file(&at, &c.fragment)?.is_some()));
            }
            Ok::<_, String>((state.components.len(), built))
        })
        .await
        else {
            continue;
        };
        let (total, built) = state;
        for (name, there) in built {
            if !there || seen.contains(&name) {
                continue;
            }
            seen.insert(name.clone());
            desk.step(&format!(
                "made the \"{name}\" micro-frontend real ({} of {total})",
                seen.len()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mockups::DirMockups;
    use crate::domain::creation::{Kind, Question};
    use crate::kit_desk;
    use crate::kit_desk::tests::Shelf;
    use crate::ports::Initialized;
    use harness_core::domain::{Halt, Outcome};
    use harness_core::ports::store::factories::Factory;
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::time::Duration;

    /// A register that keeps creations only.
    #[derive(Default)]
    struct Book(Mutex<BTreeMap<String, String>>);

    impl Factories for Book {
        fn list(&self) -> Outcome<Vec<Factory>> {
            Ok(Vec::new())
        }

        fn get(&self, _: &str) -> Outcome<Option<Factory>> {
            Ok(None)
        }

        fn enroll(&self, _: &Path, _: &str, _: &str, _: &str) -> Outcome<Factory> {
            Err(Halt::Failed("read only".to_string()))
        }

        fn creation(&self, id: &str) -> Outcome<Option<String>> {
            Ok(lock(&self.0).get(id).cloned())
        }

        fn keep_creation(&self, id: &str, record: &str) -> Outcome<()> {
            lock(&self.0).insert(id.to_string(), record.to_string());
            Ok(())
        }

        fn end_creation(&self, id: &str) -> Outcome<()> {
            lock(&self.0).remove(id);
            Ok(())
        }

        fn forget(&self, id: &str) -> Outcome<()> {
            lock(&self.0).remove(id);
            Ok(())
        }
    }

    /// A foundry that makes nothing.
    struct Nothing;

    impl Foundry for Nothing {
        fn owners(&self) -> Result<Vec<String>, String> {
            Err("none".to_string())
        }

        fn create_repository(&self, _: &str, _: &str, _: bool, _: &str) -> Result<String, String> {
            Err("none".to_string())
        }

        fn make_folder(&self, _: &Path) -> Result<(), String> {
            Err("none".to_string())
        }

        fn init_repo(&self, _: &Path, _: &str, _: &str) -> Result<Initialized, String> {
            Err("none".to_string())
        }
    }

    /// A session that draws one page where it is put to work, then makes
    /// it again of one micro-frontend — or, `lazy`, lists it and never
    /// writes its fragment.
    struct Drawer {
        lazy: bool,
    }

    impl Oneshot for Drawer {
        fn ask(&self, _: &str, prompt: &str) -> Result<String, String> {
            if prompt.contains("You guard a vertical-slice architecture") {
                assert!(prompt.contains("- `Home` —"), "the cut is in the prompt");
                return Ok(r#"{"findings":[{"check":"write_as_read","units":["ledger.saveLine"],"why":"it saves","fix":"move it to actions"}],"effects":{}}"#.to_string());
            }
            if prompt.contains("You draft the data model") {
                assert!(
                    prompt.contains("`fixtures/Home.json`"),
                    "the example data is in the prompt"
                );
                return Ok(r#"{"tables":[{"name":"line","purpose":"a line","fields":[{"name":"id","type":"uuid"}]}],"aggregates":[{"aggregate":"Line","fields":{"line.id":{"kind":"authoritative"}},"invariants":[{"id":"every-line","rule":"shown","onViolation":"reject","rules":[1]}]}]}"#.to_string());
            }
            if prompt.contains("You keep the data model") {
                assert!(prompt.contains("The change asked:\n\nadd a total"));
                return Ok(r#"{"tables":[{"name":"line","fields":[{"name":"id","type":"uuid"},{"name":"total","type":"numeric"}]}],"aggregates":[{"aggregate":"Line","fields":{"line.id":{"kind":"authoritative"}},"invariants":[{"id":"every-line","rule":"shown","rules":[1]}]}]}"#.to_string());
            }
            if prompt.contains("Read again this one requirement only") {
                return Ok(r#"{"requirements":[{"title":"See every line","family":"business","criterion":"given lines, when opened, then all show","pages":["pages/home.html"]}]}"#.to_string());
            }
            if prompt.contains("read the requirements") && prompt.contains("- R7: Mine\n") {
                return Ok(r#"{"requirements":[{"title":"See the ledger","body":"the lines","pages":["pages/home.html"],"components":["Home","Ghost"]}],"ties":[{"id":7,"pages":["pages/home.html"],"components":["Home"]}]}"#.to_string());
            }
            if prompt.contains("read the requirements") {
                return Ok(r#"{"requirements":[{"title":"See the ledger","body":"the lines","pages":["pages/home.html"],"components":["Home","Ghost"]}]}"#.to_string());
            }
            Ok("## Goal\nfirst".to_string())
        }

        fn work_in(&self, model: &str, prompt: &str, dir: &Path) -> Result<String, String> {
            let write = |path: &str, text: &str| -> Result<(), String> {
                let full = dir.join(path);
                std::fs::create_dir_all(full.parent().ok_or("no folder")?)
                    .map_err(|e| e.to_string())?;
                std::fs::write(full, text).map_err(|e| e.to_string())
            };
            assert!(
                prompt.contains("the branch `main_agent`"),
                "the branch is in the prompt"
            );
            // The mock-up is drawn and cut by the drawing's model; a guard's
            // fix is the creation's own.
            let drawing = !prompt.contains("The architecture's guard found this");
            assert_eq!(
                model,
                if drawing { "d" } else { "m" },
                "the model a session works with"
            );
            if prompt.contains("The architecture's guard found this") {
                assert!(prompt.contains("- about: ledger.saveLine"));
                return Ok("moved ledger.saveLine to actions".to_string());
            }
            if prompt.contains("Make it again of **micro-frontends**") {
                assert!(prompt.contains("a ledger"), "the brief is in the prompt");
                if !self.lazy {
                    write("mock/components/Home.html", "<section>home</section>")?;
                }
                write(
                    "mock/pages/home.html",
                    r#"<div data-component="Home"></div>"#,
                )?;
                write("mock/fixtures/Home.json", r#"[{"line": 1}]"#)?;
                write(
                    "mock/pages.json",
                    r#"{"version":1,"pages":[{"path":"pages/home.html","title":"Home","components":["Home"]}],"components":[{"name":"Home","fragment":"components/Home.html","fixture":"fixtures/Home.json"}]}"#,
                )?;
                return Ok("one micro-frontend: Home".to_string());
            }
            assert!(
                prompt.contains("## Goal\nproof"),
                "the scope is in the prompt"
            );
            assert!(
                prompt.contains("--color-primary:#2b6cb0"),
                "the design system's variables are in the prompt"
            );
            // A session that copies the design system and reaches for a
            // variable it has not: the copy goes, the variable is said.
            write("mock/design/tokens.css", ":root{--made-up:red;}")?;
            write(
                "mock/pages/home.html",
                r#"<main style="color: var(--color-primary); background: var(--made-up)">home</main>"#,
            )?;
            write(
                "mock/pages.json",
                r#"{"version":1,"pages":[{"path":"pages/home.html","title":"Home"}],"components":[]}"#,
            )?;
            Ok("one page: home".to_string())
        }
    }

    /// Pins on the mock-up's boards: one request left, one on the backlog
    /// already, one deleted.
    struct Pins;

    impl NoteStore for Pins {
        fn boards(&self) -> Result<Vec<String>, String> {
            Ok(vec![
                "mockup--main-agent--home".to_string(),
                "mockup--other-branch--home".to_string(),
                "data-model".to_string(),
            ])
        }
        fn notes(&self, board: &str) -> Result<Vec<crate::ports::KeptNote>, String> {
            use crate::domain::annotations::Note;
            // Another branch's mock-up and the data model carry pins too:
            // none of them belongs to this factory's mock-up.
            if board == "mockup--other-branch--home" || board == "data-model" {
                return Ok(vec![crate::ports::KeptNote {
                    note: Note {
                        id: "foreign".to_string(),
                        text: "another board's request".to_string(),
                        kind: PinKind::Request,
                        ..Note::default()
                    },
                    deleted_at: None,
                }]);
            }
            if board != "mockup--main-agent--home" {
                return Ok(Vec::new());
            }
            let pin = |kind: PinKind, text: &str, issue: &str, gone: bool| crate::ports::KeptNote {
                note: Note {
                    id: text.to_string(),
                    target: "p1".to_string(),
                    text: text.to_string(),
                    kind,
                    issue: issue.to_string(),
                    ..Note::default()
                },
                deleted_at: gone.then(|| "now".to_string()),
            };
            Ok(vec![
                pin(PinKind::Request, "the total above the lines", "", false),
                pin(PinKind::Request, "on the backlog", "https://x/1", false),
                pin(PinKind::Request, "deleted", "", true),
                pin(PinKind::Note, "a note", "", false),
            ])
        }
        fn put(&self, _: &str, _: &crate::domain::annotations::Note) -> Result<(), String> {
            Ok(())
        }
        fn mark_deleted(&self, _: &str, _: &str, _: Option<&str>) -> Result<(), String> {
            Ok(())
        }
        fn destroy(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn conversation(&self, _: &str, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn save_conversation(&self, _: &str, _: &str, _: &str, _: usize) -> Result<(), String> {
            Ok(())
        }
    }

    /// A session that works slowly, then writes nothing.
    struct Slow;

    impl Oneshot for Slow {
        fn ask(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }

        fn work_in(&self, _: &str, _: &str, _: &Path) -> Result<String, String> {
            std::thread::sleep(Duration::from_millis(300));
            Ok("late".to_string())
        }
    }

    /// An AI that only answers: it cannot be put to work.
    struct Talker;

    impl Oneshot for Talker {
        fn ask(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
    }

    /// An AI that sorts a description into the brief, saying the
    /// description it was given back as the first part.
    struct Sorter;

    impl Oneshot for Sorter {
        fn ask(&self, _: &str, prompt: &str) -> Result<String, String> {
            let (_, said) = prompt
                .split_once("The description:\n\n")
                .ok_or("no description")?;
            Ok(serde_json::json!({ "what": said.trim(), "who": "bakers" }).to_string())
        }
    }

    /// An AI that rewrites R1 when told the micro-frontend `Ai` changed.
    struct Proposer;

    impl Oneshot for Proposer {
        fn ask(&self, _: &str, prompt: &str) -> Result<String, String> {
            if !prompt.contains("What changed: Ai changed on every page that shows it.") {
                return Err("not told the change".to_string());
            }
            Ok(
                r#"{"proposals": [{"id": 1, "why": "w", "title": "Rewritten",
                "criterion": {"given": "g", "when": "w", "then": "t"}}]}"#
                    .to_string(),
            )
        }
    }

    #[tokio::test]
    async fn a_change_of_the_mock_up_proposes_rules_kept_until_settled() {
        let book = Arc::new(Book::default());
        let keep = |record: &Creation| {
            book.keep_creation("alpha", &serde_json::to_string(record).expect("json"))
                .expect("kept");
        };
        let mut record = Creation {
            step: Step::Rules,
            ..Creation::new("alpha", "me/alpha")
        };
        keep(&record);
        let desk = CreationDesk::new(Setup {
            register: Arc::clone(&book) as Arc<dyn Factories + Send + Sync>,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: PathBuf::from("/factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: Some(Arc::new(Proposer)),
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: None,
            kit: None,
            data: None,
            plant: None,
            pins: None,
        });
        let change = RuleChange {
            name: "Ai".to_string(),
            saved: "Ai".to_string(),
            how: "replace".to_string(),
            ..RuleChange::default()
        };
        let none = desk
            .propose_rules(change.clone())
            .await
            .expect("no rule yet");
        assert!(
            none.proposals.is_empty(),
            "no rule: nothing to keep in step"
        );

        record.requirements = vec![crate::domain::creation::Requirement {
            id: 1,
            title: "Old".to_string(),
            ..crate::domain::creation::Requirement::default()
        }];
        record.last_requirement = 1;
        keep(&record);
        let proposed = desk.propose_rules(change).await.expect("proposed");
        assert_eq!(
            proposed
                .proposals
                .iter()
                .map(|p| (p.id, p.title.as_str()))
                .collect::<Vec<_>>(),
            [(1, "Rewritten")]
        );
        let key = proposed.proposals[0].key;
        let settled = desk.settle_proposal(key, true).await.expect("accepted");
        assert_eq!(settled.requirements[0].title, "Rewritten");
        assert!(settled.proposals.is_empty());
        assert!(
            desk.settle_proposal(key, true).await.is_err(),
            "settled once"
        );
        assert!(
            desk.propose_rules(RuleChange::default()).await.is_err(),
            "an answer that is no proposal"
        );
    }

    #[tokio::test]
    async fn a_description_is_kept_then_sorted_into_the_brief() {
        let book = Arc::new(Book::default());
        let record = Creation {
            brief: "### Who uses it?\n\nold".to_string(),
            step: Step::Brief,
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let desk = |oneshot: Option<Arc<dyn Oneshot>>| {
            CreationDesk::new(Setup {
                register: Arc::clone(&book) as Arc<dyn Factories + Send + Sync>,
                factory: "alpha".to_string(),
                foundry: Arc::new(Nothing),
                factories_dir: PathBuf::from("/factories"),
                integration_branch: "main_agent".to_string(),
                oneshot,
                model: "m".to_string(),
                draw_model: "d".to_string(),
                mockups: None,
                kit: None,
                data: None,
                plant: None,
                pins: None,
            })
        };
        let closed = desk(Some(Arc::new(Sorter)))
            .decompose("a ledger".to_string())
            .await;
        assert_eq!(
            closed.map(|_| ()),
            Err("choose the kind of factory".to_string())
        );

        let kind = Edit {
            kind: Some(Kind::Vertical),
            ..Edit::default()
        };
        desk(None).save(kind, None).await.expect("saved");
        let blank = desk(Some(Arc::new(Sorter)))
            .decompose(" ".to_string())
            .await;
        assert_eq!(
            blank.map(|_| ()),
            Err("describe the application first".to_string())
        );
        let mute = desk(None).decompose("a ledger".to_string()).await;
        assert_eq!(mute.map(|_| ()), Err(NO_CLAUDE.to_string()));
        let kept = desk(None).record().expect("read").expect("a record");
        assert_eq!(
            (kept.description.as_str(), kept.brief.as_str()),
            ("a ledger", "### Who uses it?\n\nold"),
            "the description kept even when nothing sorts it"
        );

        let sorted = desk(Some(Arc::new(Sorter)))
            .decompose("a bakery's ledger".to_string())
            .await
            .expect("sorted");
        assert_eq!(sorted.description, "a bakery's ledger");
        assert_eq!(
            sorted.brief,
            "### What is the application, and what problem does it solve?\n\na bakery's ledger\n\n### Who uses it?\n\nbakers"
        );
        assert_eq!(desk(None).record().expect("read"), Some(sorted));
    }

    #[test]
    fn an_ai_that_only_answers_refuses_to_write() {
        assert!(Talker.work_in("m", "p", Path::new("/")).is_err());
    }

    fn product(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "harness-creation-desk-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |path: &str, text: &str| {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().expect("a folder")).expect("folder");
            std::fs::write(full, text).expect("write");
        };
        write(
            "main_agent/mock/pages.json",
            r#"{"version":1,"pages":[],"components":[]}"#,
        );
        write(
            "design-system/mock/design/design.json",
            r#"{"version":1,"state":"validated"}"#,
        );
        write(
            "design-system/mock/design/tokens.css",
            ":root{--color-primary:#2b6cb0;}\n",
        );
        write(
            "design-system/mock/kit/kit.json",
            r#"{"version":1,"checked":[],"components":[]}"#,
        );
        dir
    }

    async fn settled(desk: &CreationDesk) -> Job {
        for _ in 0..200 {
            if let Some(job) = desk.job().filter(|j| j.state != State::Running) {
                return job;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        desk.job().expect("a job")
    }

    // One story: the gates read the design system, the mock-ups are drawn
    // in the branch's workspace, the review opens, the initialization runs
    // until the board it cannot reach.
    #[tokio::test]
    async fn a_proof_of_concept_is_drawn_reviewed_and_initialized_up_to_the_board() {
        let dir = product("story");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let kit = KitDesk::new(kit_desk::Setup {
            mockups: Arc::clone(&mockups),
            oneshot: None,
            model: "m".into(),
            sessions: 1,
            pins: Arc::new(Shelf::default()),
            integration_branch: "main_agent".into(),
        });
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            questions: vec![Question {
                id: 1,
                title: "who".to_string(),
                answer: "the client".to_string(),
                ..Question::default()
            }],
            scope: "## Goal\nproof".to_string(),
            step: Step::Mockups,
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: Some(Arc::new(Drawer { lazy: false })),
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: Some(mockups),
            kit: Some(kit),
            data: None,
            plant: None,
            pins: Some(Arc::new(Pins)),
        });
        assert!(desk.facts().await.design_validated);
        assert!(desk.accept().await.is_err(), "nothing drawn yet");
        assert!(desk.componentize_now().await.is_err(), "nothing drawn yet");
        assert!(desk.job().is_none(), "a refused acceptance starts nothing");

        desk.draw_mockups().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(
            (job.work, job.state),
            (Work::Mockups, State::Done),
            "{}",
            job.why
        );
        assert!(dir.join("main_agent/mock/pages/home.html").is_file());
        kept_on_the_design_system(&job, &dir);
        let drawn = desk.record().expect("read").expect("a record");
        assert_eq!(drawn.mocked.as_deref(), Some("one page: home"));
        assert_eq!(drawn.step, Step::Rules);

        // The micro-frontends before the review, so the requirements tie to them.
        desk.componentize_now().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(
            (job.work, job.state),
            (Work::Componentize, State::Done),
            "{}",
            job.why
        );
        assert!(
            job.steps
                .iter()
                .any(|s| s == "every page is made of micro-frontends"),
            "{:?}",
            job.steps
        );
        assert_eq!(
            desk.record()
                .expect("read")
                .expect("a record")
                .componentized
                .as_deref(),
            Some("one micro-frontend: Home")
        );
        desk.componentize_now()
            .await
            .expect("done already: nothing runs");
        let id = rules_read_the_human_s_tied_again(&desk).await;
        rules_read_again_and_guarded(&desk, id).await;
        data_drafted_and_retouched(&desk).await;
        initialized_up_to_the_board(&desk, &dir).await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session that draws like [`Drawer`] but writes a micro-frontend's
    /// Capabilities as one string where the contract wants a list — and,
    /// `repairs`, puts it right when asked.
    struct Sloppy {
        repairs: bool,
    }

    const UNREAD: &str = r#"{"version":1,"pages":[{"path":"pages/home.html","title":"Home"}],"components":[{"name":"Home","needs":"ledger.lines"}]}"#;

    impl Oneshot for Sloppy {
        fn ask(&self, model: &str, prompt: &str) -> Result<String, String> {
            Drawer { lazy: false }.ask(model, prompt)
        }

        fn work_in(&self, model: &str, prompt: &str, dir: &Path) -> Result<String, String> {
            let manifest = dir.join("mock/pages.json");
            if prompt.contains("cannot read the mock-up") {
                assert!(
                    prompt.contains("invalid type"),
                    "the error is in the prompt"
                );
                if self.repairs {
                    std::fs::write(
                        &manifest,
                        UNREAD.replace(r#""ledger.lines""#, r#"["ledger.lines"]"#),
                    )
                    .map_err(|e| e.to_string())?;
                }
                return Ok("repaired".to_string());
            }
            let said = Drawer { lazy: false }.work_in(model, prompt, dir)?;
            if !prompt.contains("Make it again of **micro-frontends**") {
                std::fs::write(&manifest, UNREAD).map_err(|e| e.to_string())?;
            }
            Ok(said)
        }
    }

    // A manifest the view cannot read goes back to a session: repaired, the
    // job goes on; never repaired, it stops on it instead of ending done.
    #[tokio::test]
    async fn a_manifest_that_does_not_read_is_repaired_or_stops_the_job() {
        for repairs in [true, false] {
            let dir = product(if repairs { "repaired" } else { "unrepaired" });
            let book = Arc::new(Book::default());
            let record = Creation {
                kind: Some(Kind::Vertical),
                brief: "a ledger".to_string(),
                grilled: true,
                scope: "## Goal\nproof".to_string(),
                step: Step::Design,
                ..Creation::new("alpha", "me/alpha")
            };
            book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
                .expect("kept");
            let desk = CreationDesk::new(Setup {
                register: book,
                factory: "alpha".to_string(),
                foundry: Arc::new(Nothing),
                factories_dir: dir.join("factories"),
                integration_branch: "main_agent".to_string(),
                oneshot: Some(Arc::new(Sloppy { repairs })),
                model: "m".to_string(),
                draw_model: "d".to_string(),
                mockups: Some(Arc::new(DirMockups::new(&dir))),
                kit: None,
                data: None,
                plant: None,
                pins: Some(Arc::new(Pins)),
            });
            desk.draw_ahead().await.expect("started");
            let job = settled(&desk).await;
            let asked = job
                .steps
                .iter()
                .filter(|s| s.ends_with("a session repairs it"))
                .count();
            let kept = desk.record().expect("read").expect("a record");
            if repairs {
                assert_eq!(job.state, State::Done, "{}", job.why);
                assert_eq!(asked, 1, "{:?}", job.steps);
                assert!(kept.mocked.is_some());
            } else {
                assert_eq!((job.work, job.state), (Work::Mockups, State::Failed));
                assert!(job.why.contains("invalid type"), "{}", job.why);
                assert_eq!(asked, MANIFEST_REPAIRS as usize, "{:?}", job.steps);
                assert_eq!(kept.mocked, None, "an unreadable drawing is not kept");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    // The scope settled, the proof of concept draws ahead while the human is
    // on the design system: mock-ups, micro-frontends, requirements — the
    // human's step left where it is.
    #[tokio::test]
    async fn the_proof_of_concept_draws_ahead_once_the_scope_is_settled() {
        let dir = product("ahead");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            grilled: true,
            step: Step::Design,
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: Some(Arc::new(Drawer { lazy: false })),
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: Some(mockups),
            kit: None,
            data: None,
            plant: None,
            pins: Some(Arc::new(Pins)),
        });
        assert_eq!(
            desk.draw_ahead().await,
            Err("settle the proof of concept's scope".to_string()),
            "no scope, nothing to draw"
        );
        assert!(desk.job().is_none());

        desk.save(
            Edit {
                scope: Some("## Goal\nproof".to_string()),
                ..Edit::default()
            },
            None,
        )
        .await
        .expect("saved");
        desk.draw_ahead().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(
            (job.work, job.state),
            (Work::Componentize, State::Done),
            "{}",
            job.why
        );
        assert_eq!(
            job.steps.last().map(String::as_str),
            Some("1 requirement(s) read")
        );
        let ahead = desk.record().expect("read").expect("a record");
        assert_eq!(ahead.step, Step::Design, "the human's step is left");
        assert_eq!(ahead.mocked.as_deref(), Some("one page: home"));
        assert_eq!(ahead.mocked_scope, "## Goal\nproof");
        assert_eq!(
            ahead.componentized.as_deref(),
            Some("one micro-frontend: Home")
        );
        assert_eq!(ahead.requirements.len(), 1);
        assert!(!ahead.mockup_stale());

        // Drawn already: nothing runs again.
        desk.draw_ahead().await.expect("nothing to do");
        assert_eq!(desk.job().map(|j| j.steps), Some(job.steps));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The rules read off the mock-up, a rule of the human's tied to a page
    /// an earlier drawing had kept and tied again. Answers the AI's rule.
    async fn rules_read_the_human_s_tied_again(desk: &CreationDesk) -> u32 {
        desk.save(
            Edit {
                requirements: Some(vec![crate::domain::creation::Requirement {
                    id: 7,
                    title: "Mine".to_string(),
                    human: true,
                    criterion: creation::Criterion {
                        given: "lines".to_string(),
                        when: "opened".to_string(),
                        then: "all show".to_string(),
                    },
                    pages: vec!["pages/gone.html".to_string()],
                    ..crate::domain::creation::Requirement::default()
                }]),
                ..Edit::default()
            },
            None,
        )
        .await
        .expect("the human's rule");
        let read = desk.read_requirements().await.expect("read");
        assert_eq!(read.requirements.len(), 2);
        let mine = &read.requirements[0];
        assert_eq!((mine.title.as_str(), mine.human), ("Mine", true));
        assert_eq!(mine.pages, ["pages/home.html"], "tied again");
        assert_eq!(mine.components, ["Home"]);
        let ais = &read.requirements[1];
        assert_eq!(ais.title, "See the ledger");
        assert_eq!(ais.pages, vec!["pages/home.html".to_string()]);
        assert_eq!(
            ais.components,
            vec!["Home".to_string()],
            "a micro-frontend the mock-up has not is dropped"
        );
        ais.id
    }

    /// The third quarter of the flow above: one rule read again, then the
    /// architecture's guard — a write passed off as a read blocks the
    /// rules until its fix is applied.
    async fn rules_read_again_and_guarded(desk: &CreationDesk, id: u32) {
        let again = desk.reread_rule(id).await.expect("read again");
        let rule = again
            .requirements
            .iter()
            .find(|r| r.id == id)
            .expect("the rule");
        assert_eq!((rule.id, rule.title.as_str()), (id, "See every line"));
        assert_eq!(rule.family, creation::Family::Business);
        assert!(desk.reread_rule(id + 7).await.is_err(), "no such rule");

        let audited = desk.audit().await.expect("audited");
        let audit = audited.audit.as_ref().expect("an audit");
        assert_eq!(audit.from, "one micro-frontend: Home");
        assert_eq!(audit.blocking().len(), 1);
        assert!(
            audited
                .rule_gaps()
                .iter()
                .any(|g| g == "architecture: it saves")
        );
        let blocking = audit.findings[0].id;
        assert!(
            desk.resolve_finding(blocking, false).await.is_err(),
            "fixed, not dismissed"
        );
        assert!(
            desk.resolve_finding(blocking + 9, true).await.is_err(),
            "no such finding"
        );
        let fixed = desk.resolve_finding(blocking, true).await.expect("applied");
        assert!(fixed.audit.expect("kept").findings.is_empty());
    }

    /// The data step of the flow above: drafted, its structure changed by
    /// the AI, its criticality picked by the human, validated.
    async fn data_drafted_and_retouched(desk: &CreationDesk) {
        use crate::domain::data_draft::{Criticality, CriticalityEdit, DataEdit, TextEdit};
        assert!(
            desk.draft_data().await.is_err(),
            "the rules are not validated yet"
        );
        assert!(
            desk.change_data("anything").await.is_err(),
            "no data model yet"
        );
        let mut record = desk.existing().expect("a creation");
        // R1, the AI's: the human's R7 is kept before it.
        let first = record
            .requirements
            .iter()
            .position(|r| r.id == 1)
            .expect("R1");
        record.requirements[first].criterion =
            creation::Criterion::from_sentence("given lines, when opened, then all show");
        record.requirements[first]
            .pages
            .push("pages/gone.html".to_string());
        desk.keep(&record).expect("kept");
        let refused = desk
            .validate_rules()
            .await
            .expect_err("a tie to a page gone");
        assert!(
            refused.contains("R1 points at pages/gone.html, which the mock-up has not"),
            "{refused}"
        );
        record.requirements[first].pages.pop();
        desk.keep(&record).expect("kept");
        assert!(
            desk.validate_rules()
                .await
                .expect("validated")
                .rules_validated
        );

        let drafted = desk.draft_data().await.expect("drafted");
        let data = drafted.data.as_ref().expect("a draft");
        assert_eq!(data.tables[0].name, "line");
        assert_eq!(data.from, rules_print(&drafted));
        assert_eq!(
            drafted.data_gaps(),
            vec!["Line: line.id has no criticality".to_string()]
        );
        assert!(desk.validate_data().await.is_err());
        assert!(desk.change_data("  ").await.is_err(), "nothing asked");

        let changed = desk.change_data("add a total").await.expect("changed");
        assert_eq!(
            changed.data.as_ref().expect("a draft").tables[0]
                .fields
                .len(),
            2
        );
        let edited = desk
            .edit_data(DataEdit {
                texts: vec![TextEdit {
                    table: "line".to_string(),
                    field: "total".to_string(),
                    purpose: "the sum".to_string(),
                }],
                criticalities: vec![CriticalityEdit {
                    aggregate: "Line".to_string(),
                    field: "line.id".to_string(),
                    criticality: Criticality::Normal,
                }],
                ..DataEdit::default()
            })
            .await
            .expect("edited");
        assert_eq!(
            edited.data.as_ref().expect("a draft").tables[0].fields[1].purpose,
            "the sum"
        );
        let valid = desk.validate_data().await.expect("validated");
        assert!(valid.data_validated());
    }

    /// The rules and the data model the acceptance waits for, validated:
    /// the rules read given a criterion and a support where they lack one,
    /// or one of its own; a data model of one table.
    fn rules_validated(desk: &CreationDesk) {
        use crate::domain::data_draft::{Aggregate, Criticality, DataDraft, FieldRule, Table};
        let mut record = desk.existing().expect("a creation");
        if record.requirements.is_empty() {
            record.requirements.push(creation::Requirement {
                id: 1,
                title: "Pay".to_string(),
                ..creation::Requirement::default()
            });
        }
        for r in &mut record.requirements {
            if r.criterion.is_empty() {
                r.criterion =
                    creation::Criterion::from_sentence("given a debt, when paid, then settled");
            }
            if !r.covered() {
                r.components.push("InvoiceList".to_string());
            }
        }
        record.validate_rules().expect("nothing keeps them");
        let business: Vec<u32> = record
            .requirements
            .iter()
            .filter(|r| r.family == creation::Family::Business)
            .map(|r| r.id)
            .collect();
        record.data = Some(DataDraft {
            tables: vec![Table {
                name: "line".to_string(),
                ..Table::default()
            }],
            aggregates: vec![Aggregate {
                name: "Line".to_string(),
                fields: std::collections::BTreeMap::from([(
                    "line.total".to_string(),
                    FieldRule {
                        criticality: Some(Criticality::Normal),
                        rules: business,
                        ..FieldRule::default()
                    },
                )]),
                ..Aggregate::default()
            }],
            from: rules_print(&record),
            ..DataDraft::default()
        });
        record.validate_data().expect("nothing keeps it");
        desk.keep(&record).expect("kept");
    }

    /// What the story's drawing left of the design system: its copy gone,
    /// the variable it has not said, the job's time kept.
    fn kept_on_the_design_system(job: &Job, dir: &Path) {
        for step in [
            "1 file(s) the session wrote under design/ and kit/ removed: the design system's own are served (design/tokens.css)",
            "variables the design system has not, left to put right: --made-up",
        ] {
            assert!(
                job.steps.iter().any(|s| s == step),
                "{step}: {:?}",
                job.steps
            );
        }
        assert!(!dir.join("main_agent/mock/design/tokens.css").exists());
        assert!(
            job.started > 1_700_000_000 && job.ended >= job.started,
            "a job keeps when it started and ended, on the clock"
        );
    }

    // The design system's interface examples a drawing reads are lent beside
    // its pages, the component's left to the kit; taken back unreported,
    // what the session took from each said.
    #[tokio::test]
    async fn interface_examples_are_lent_to_a_drawing_and_taken_back() {
        let dir = product("examples");
        let write = |rel: &str, bytes: &[u8]| {
            let full = dir.join(rel);
            std::fs::create_dir_all(full.parent().expect("a folder")).expect("folder");
            std::fs::write(full, bytes).expect("write");
        };
        write(
            "design-system/mock/design/design.json",
            br#"{"version":1,"state":"validated","examples":[
                {"file":"examples/app.png","note":"side nav"},
                {"file":"examples/login.png","scope":{"on":"page","name":"Login"}},
                {"file":"examples/table.png","scope":{"on":"component","name":"Table"}},
                {"file":"examples/gone.png"}]}"#,
        );
        for f in ["app", "login", "table"] {
            write(
                &format!("design-system/mock/design/examples/{f}.png"),
                b"png",
            );
        }
        let desk = desk_drawing_in(&dir);
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let lent = desk.examples_lent(&mockups, "main_agent").await;
        assert_eq!(
            lent.iter().map(|e| e.file.as_str()).collect::<Vec<_>>(),
            ["examples/app.png", "examples/login.png"],
            "the component's goes to the kit, a missing image is not lent"
        );
        assert!(dir.join("main_agent/mock/design/examples/app.png").exists());
        assert!(
            !dir.join("main_agent/mock/design/examples/table.png")
                .exists()
        );
        desk.said_of_examples("done\n- example examples/app.png: the side nav");
        desk.kept_on_design(&mockups, "main_agent", "").await;
        assert!(!dir.join("main_agent/mock/design/examples/app.png").exists());
        let steps = desk.job().expect("a job").steps;
        assert!(
            steps.contains(
                &"2 interface example(s) of the design system lent to the session".to_string()
            ),
            "{steps:?}"
        );
        assert!(
            steps.contains(&"example examples/app.png: the side nav".to_string()),
            "{steps:?}"
        );
        assert!(
            !steps.iter().any(|s| s.contains("removed")),
            "the examples lent are not reported: {steps:?}"
        );
        // A design system with none: nothing lent, nothing said.
        let bare = product("no-examples");
        let desk = desk_drawing_in(&bare);
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&bare));
        assert!(desk.examples_lent(&mockups, "main_agent").await.is_empty());
        assert!(desk.job().map(|j| j.steps).unwrap_or_default().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bare);
    }

    // With no design system's variables to hold them to, a session's
    // variables are not judged; its copies under design/ and kit/ still go.
    #[tokio::test]
    async fn no_variable_is_judged_before_the_design_system_has_any() {
        let dir = product("no-tokens");
        std::fs::create_dir_all(dir.join("main_agent/mock/pages")).expect("dir");
        std::fs::write(
            dir.join("main_agent/mock/pages/a.html"),
            "<p style=\"color: var(--anything)\"></p>",
        )
        .expect("page");
        std::fs::create_dir_all(dir.join("main_agent/mock/kit")).expect("dir");
        std::fs::write(dir.join("main_agent/mock/kit/Copy.html"), "<b></b>").expect("copy");
        let desk = desk_drawing_in(&dir);
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        desk.kept_on_design(&mockups, "main_agent", "").await;
        let steps = desk.job().expect("a job").steps;
        assert!(
            !steps
                .iter()
                .any(|s| s.starts_with("variables the design system has not")),
            "{steps:?}"
        );
        assert!(
            steps.iter().any(
                |s| s.starts_with("1 file(s) the session wrote under design/ and kit/ removed")
            ),
            "{steps:?}"
        );
        assert!(!dir.join("main_agent/mock/kit/Copy.html").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A creation desk with a drawing under way, over the mock-ups in `dir`.
    fn desk_drawing_in(dir: &Path) -> CreationDesk {
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(dir));
        let book = Arc::new(Book::default());
        book.keep_creation(
            "alpha",
            &serde_json::to_string(&Creation::new("alpha", "me/alpha")).expect("json"),
        )
        .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: None,
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: Some(mockups),
            kit: None,
            data: None,
            plant: None,
            pins: None,
        });
        desk.begin(Work::Mockups).expect("nothing under way");
        desk
    }

    /// Pins on two boards of this branch's mock-up and one of another
    /// branch, one already deleted on each: what is put away is noted.
    #[derive(Default)]
    struct Shelved(Mutex<Vec<(String, String)>>);

    impl NoteStore for Shelved {
        fn boards(&self) -> Result<Vec<String>, String> {
            Ok(vec![
                "mockup--main-agent--home".to_string(),
                "mockup--main-agent--about".to_string(),
                "mockup--other--home".to_string(),
            ])
        }
        fn notes(&self, board: &str) -> Result<Vec<crate::ports::KeptNote>, String> {
            use crate::domain::annotations::Note;
            let pin = |id: &str, gone: bool| crate::ports::KeptNote {
                note: Note {
                    id: format!("{board}/{id}"),
                    ..Note::default()
                },
                deleted_at: gone.then(|| "before".to_string()),
            };
            Ok(vec![pin("on", false), pin("gone", true)])
        }
        fn put(&self, _: &str, _: &crate::domain::annotations::Note) -> Result<(), String> {
            Ok(())
        }
        fn mark_deleted(&self, board: &str, id: &str, at: Option<&str>) -> Result<(), String> {
            assert!(
                at.is_some_and(|a| a.starts_with("20")),
                "put away at a time: {at:?}"
            );
            lock(&self.0).push((board.to_string(), id.to_string()));
            Ok(())
        }
        fn destroy(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn conversation(&self, _: &str, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn save_conversation(&self, _: &str, _: &str, _: &str, _: usize) -> Result<(), String> {
            Ok(())
        }
    }

    // Before a drawing replaces the pages, the pins still on this branch's
    // mock-up are put away; another branch's and those deleted already stay.
    #[tokio::test]
    async fn a_drawing_puts_away_the_pins_of_the_mock_up_it_replaces() {
        let dir = product("pins-away");
        let shelved = Arc::new(Shelved::default());
        let book = Arc::new(Book::default());
        book.keep_creation(
            "alpha",
            &serde_json::to_string(&Creation::new("alpha", "me/alpha")).expect("json"),
        )
        .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: None,
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: None,
            kit: None,
            data: None,
            plant: None,
            pins: Some(Arc::clone(&shelved) as Arc<dyn NoteStore>),
        });
        assert_eq!(desk.pins_put_away("main_agent").await, 2);
        let mut away = lock(&shelved.0).clone();
        away.sort();
        assert_eq!(
            away,
            vec![
                (
                    "mockup--main-agent--about".to_string(),
                    "mockup--main-agent--about/on".to_string()
                ),
                (
                    "mockup--main-agent--home".to_string(),
                    "mockup--main-agent--home/on".to_string()
                ),
            ]
        );
        let bare = desk_drawing_in(&dir);
        assert_eq!(
            bare.pins_put_away("main_agent").await,
            0,
            "no pins kept here: nothing to put away"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The second half of the flow above: accepted, the record is kept in
    /// the repository, the mock-up and the design system published, and
    /// the initialization stops at the board it has not.
    async fn initialized_up_to_the_board(desk: &CreationDesk, dir: &Path) {
        rules_validated(desk);
        let accepted = desk.accept().await.expect("accepted");
        assert!(accepted.accepted);
        assert_eq!(accepted.step, Step::Initialize);
        let job = settled(desk).await;
        assert_eq!((job.work, job.state), (Work::Initialize, State::Failed));
        assert!(job.why.contains("board"), "{}", job.why);
        assert!(
            !job.steps
                .iter()
                .any(|s| s == "every page is made of micro-frontends"),
            "made already, not made again: {:?}",
            job.steps
        );
        // The record is in the repository beside the mock-up, with its context.
        let model = std::fs::read_to_string(dir.join("main_agent/mock/data/model.json"))
            .expect("the data model is written beside the mock-up");
        assert!(model.contains("\"name\": \"line\""), "{model}");
        let aggregates = std::fs::read_to_string(dir.join("main_agent/mock/data/aggregates.json"))
            .expect("the aggregates are written beside the mock-up");
        assert!(
            aggregates.contains("\"aggregate\": \"Line\""),
            "{aggregates}"
        );
        let kept = std::fs::read_to_string(dir.join("main_agent/mock/proof-of-concept.json"))
            .expect("the record is written");
        let kept: serde_json::Value = serde_json::from_str(&kept).expect("json");
        assert_eq!(kept["requirements"][0]["title"], "Mine");
        assert_eq!(kept["requirements"][1]["title"], "See every line");
        assert_eq!(kept["requirements"][1]["family"], "business");
        assert_eq!(kept["components"][0]["name"], "Home");
        assert_eq!(kept["pins"][0]["text"], "the total above the lines");
        assert_eq!(
            kept["pins"].as_array().map(Vec::len),
            Some(1),
            "{}",
            kept["pins"]
        );
        assert_eq!(kept["decisions"][0]["title"], "who");
        assert_eq!(
            kept["design"]["components"].as_array().map(Vec::len),
            Some(0),
            "no component is generated: {}",
            kept["design"]
        );
        assert!(dir.join("main_agent/mock/proof-of-concept.md").is_file());
        let after = desk.record().expect("read").expect("a record");
        assert!(
            !after.mockup_at.is_empty(),
            "where the mock-up went is kept"
        );
        assert!(
            job.steps
                .iter()
                .any(|s| s.starts_with("publishing the design system"))
        );
        assert!(
            job.steps.iter().any(|s| s.starts_with("the mock-up: ")),
            "{:?}",
            job.steps
        );
        assert_eq!(
            desk.finish().await,
            Err("initialize the factory first".to_string())
        );
    }

    #[tokio::test]
    async fn without_a_creation_nothing_is_saved_and_no_job_starts() {
        let desk = CreationDesk::new(Setup {
            register: Arc::new(Book::default()),
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: PathBuf::from("/factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: None,
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: None,
            kit: None,
            data: None,
            plant: None,
            pins: None,
        });
        assert!(!desk.facts().await.design_validated);
        assert_eq!(desk.record(), Ok(None));
        let none = Err("this factory is not being created".to_string());
        assert_eq!(desk.save(Edit::default(), None).await.map(|_| ()), none);
        assert_eq!(desk.draw_mockups().await, none);
        assert!(desk.job().is_none());
        assert!(desk.owners().await.is_err());
    }

    #[tokio::test]
    async fn a_kept_record_and_a_moving_job_beat_the_pulse() {
        let book = Arc::new(Book::default());
        let record = Creation {
            step: Step::Brief,
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let pulse = Pulse::default();
        let mut ears = pulse.subscribe();
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: PathBuf::from("/factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: None,
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: None,
            kit: None,
            data: None,
            plant: None,
            pins: None,
        })
        .with_pulse(pulse);
        let mut heard = || {
            let moved = ears.has_changed().expect("the pulse lives");
            ears.mark_unchanged();
            moved
        };
        assert!(desk.record().is_ok());
        assert!(!heard(), "a read wakes nobody");

        let edit = Edit {
            brief: Some("a ledger".to_string()),
            ..Edit::default()
        };
        desk.save(edit, None).await.expect("saved");
        assert!(heard(), "a kept record");

        desk.begin(Work::Mockups).expect("nothing under way");
        assert!(heard(), "a job begun");
        desk.step("the first page");
        assert!(heard(), "a job's step");
        desk.end(Ok(()));
        assert!(heard(), "a job ended");
    }

    #[tokio::test]
    async fn preparing_leaves_the_scope_to_the_grilling_and_says_what_the_design_system_waits_for()
    {
        let dir = product("prepare");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let kit = KitDesk::new(kit_desk::Setup {
            mockups: Arc::clone(&mockups),
            oneshot: None,
            model: "m".into(),
            sessions: 1,
            pins: Arc::new(Shelf::default()),
            integration_branch: "main_agent".into(),
        });
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            step: Step::Brief,
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: Some(Arc::new(Drawer { lazy: false })),
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: Some(mockups),
            kit: Some(kit),
            data: None,
            plant: None,
            pins: None,
        });
        desk.prepare().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(
            (job.work, job.state),
            (Work::Prepare, State::Done),
            "{}",
            job.why
        );
        assert!(
            job.steps
                .iter()
                .any(|s| s.starts_with("no Claude at hand: the design system")),
            "{:?}",
            job.steps
        );
        let prepared = desk.record().expect("read").expect("a record");
        assert!(
            prepared.scope.is_empty() && prepared.scope_from.is_empty(),
            "the scope waits for the grilling: {:?}",
            prepared.scope
        );
        assert_eq!(prepared.step, Step::Brief, "a preparation moves no step");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_boards_of_a_branchs_mock_up_share_a_prefix_named_after_it() {
        assert_eq!(
            CreationDesk::boards_of("main_agent"),
            "mockup--main-agent--"
        );
        assert_eq!(
            CreationDesk::boards_of("feature/Login Form"),
            "mockup--feature-login-form--"
        );
        assert_eq!(CreationDesk::boards_of("--"), "mockup--integration--");
        assert_eq!(CreationDesk::boards_of(""), "mockup--integration--");
    }

    /// Claude for the preparation: the tokens, a component.
    struct Prepared;

    impl Oneshot for Prepared {
        fn ask(&self, _: &str, prompt: &str) -> Result<String, String> {
            if prompt.contains("design tokens") {
                return Ok(harness_architecture::kit::default_tokens_json());
            }
            let name = prompt.split('`').nth(1).unwrap_or("X").to_string();
            Ok(format!("<div class=\"kit-x\">{name}</div>"))
        }
    }

    /// A creation at its brief, over a design system as `design` and `kit`
    /// say, with a Claude at hand: the job a *Continue* on the brief starts.
    async fn prepared_over(tag: &str, design: &str, kit: &str) -> (CreationDesk, PathBuf, Job) {
        let dir = product(tag);
        let write = |path: &str, text: &[u8]| {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().expect("a folder")).expect("folder");
            std::fs::write(full, text).expect("write");
        };
        write("design-system/mock/design/design.json", design.as_bytes());
        write("design-system/mock/kit/kit.json", kit.as_bytes());
        write("design-system/mock/design/logo.png", b"png");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let oneshot: Arc<dyn Oneshot> = Arc::new(Prepared);
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            step: Step::Brief,
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: Some(Arc::clone(&oneshot)),
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: Some(Arc::clone(&mockups)),
            kit: Some(KitDesk::new(kit_desk::Setup {
                mockups,
                oneshot: Some(oneshot),
                model: "m".into(),
                sessions: 1,
                pins: Arc::new(Shelf::default()),
                integration_branch: "main_agent".into(),
            })),
            data: None,
            plant: None,
            pins: None,
        });
        desk.prepare().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(
            (job.work, job.state),
            (Work::Prepare, State::Done),
            "{}",
            job.why
        );
        // The kit's components generate in the background: let them finish.
        for _ in 0..200 {
            if desk
                .kit()
                .await
                .is_some_and(|k| !k.running && k.waiting == 0)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        (desk, dir, job)
    }

    const NONE: &str = r#"{"version":1,"state":"none"}"#;
    const ONE_TICKED: &str = r#"{"version":1,"checked":["button"],"components":[]}"#;
    const NONE_TICKED: &str = r#"{"version":1,"checked":[],"components":[]}"#;

    fn said(job: &Job, what: &str) -> bool {
        job.steps.iter().any(|s| s.contains(what))
    }

    #[tokio::test]
    async fn the_preparation_draws_the_tokens_only_from_some_material_and_only_once() {
        // Nothing to draw from: the tokens wait, the kit does not start.
        let (desk, dir, job) = prepared_over("prep-none", NONE, ONE_TICKED).await;
        assert!(
            said(&job, "no moodboard, logo or design brief yet"),
            "{:?}",
            job.steps
        );
        assert!(
            !said(&job, "drawing the design system's tokens"),
            "{:?}",
            job.steps
        );
        assert!(
            desk.kit()
                .await
                .is_some_and(|k| !k.validated && k.generated == 0)
        );
        let _ = std::fs::remove_dir_all(dir);

        // A design brief alone is material: the tokens are drawn, the ticked
        // component queues behind them and is generated, from the candidate.
        let (desk, dir, job) = prepared_over(
            "prep-brief",
            r#"{"version":1,"state":"none","brief":"calm and warm"}"#,
            ONE_TICKED,
        )
        .await;
        assert!(
            said(&job, "drawing the design system's tokens"),
            "{:?}",
            job.steps
        );
        assert!(said(&job, "1 kit component(s) queued"), "{:?}", job.steps);
        assert!(!said(&job, "no moodboard"), "{:?}", job.steps);
        let kit = desk.kit().await.expect("a kit");
        assert_eq!((kit.validated, kit.generated, kit.waiting), (false, 1, 0));
        // What the roadmap will carry of it: the tokens, and the generated components only.
        let record = desk.record().expect("read").expect("a record");
        let ctx = desk.context(&record).await;
        assert!(ctx.design.tokens > 0, "{:?}", ctx.design);
        assert_eq!(ctx.design.components, vec!["Button".to_string()]);
        let _ = std::fs::remove_dir_all(dir);

        // A logo alone is material too.
        let (_, dir, job) = prepared_over(
            "prep-logo",
            r#"{"version":1,"state":"none","logo":"logo.png"}"#,
            ONE_TICKED,
        )
        .await;
        assert!(
            said(&job, "drawing the design system's tokens"),
            "{:?}",
            job.steps
        );
        let _ = std::fs::remove_dir_all(dir);

        // A moodboard alone, as well.
        let (_, dir, job) = prepared_over(
            "prep-mood",
            r#"{"version":1,"state":"none","moodboard":["logo.png"]}"#,
            ONE_TICKED,
        )
        .await;
        assert!(
            said(&job, "drawing the design system's tokens"),
            "{:?}",
            job.steps
        );
        let _ = std::fs::remove_dir_all(dir);

        // Tokens there already: never drawn again, the kit queues behind them.
        let (_, dir, job) = prepared_over(
            "prep-drawn",
            r#"{"version":1,"state":"validated","brief":"calm"}"#,
            ONE_TICKED,
        )
        .await;
        assert!(
            said(&job, "the design system has its tokens already"),
            "{:?}",
            job.steps
        );
        assert!(
            !said(&job, "drawing the design system's tokens"),
            "{:?}",
            job.steps
        );
        assert!(said(&job, "1 kit component(s) queued"), "{:?}", job.steps);
        let _ = std::fs::remove_dir_all(dir);

        // A candidate that waits counts as drawn too — and nothing ticked, nothing queued.
        let (_, dir, job) = prepared_over(
            "prep-candidate",
            r#"{"version":1,"state":"none","candidate":true,"brief":"calm"}"#,
            NONE_TICKED,
        )
        .await;
        assert!(
            said(&job, "the design system has its tokens already"),
            "{:?}",
            job.steps
        );
        assert!(!said(&job, "queued"), "{:?}", job.steps);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_shutdown_stops_the_job_under_way() {
        let dir = product("shutdown");
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "b".to_string(),
            questions: vec![Question {
                id: 1,
                title: "t".to_string(),
                answer: "a".to_string(),
                ..Question::default()
            }],
            scope: "s".to_string(),
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let make = || {
            CreationDesk::new(Setup {
                register: book.clone(),
                factory: "alpha".to_string(),
                foundry: Arc::new(Nothing),
                factories_dir: dir.join("factories"),
                integration_branch: "main_agent".to_string(),
                oneshot: Some(Arc::new(Slow)),
                model: "m".to_string(),
                draw_model: "d".to_string(),
                mockups: Some(Arc::clone(&mockups)),
                kit: Some(KitDesk::new(kit_desk::Setup {
                    mockups: Arc::clone(&mockups),
                    oneshot: None,
                    model: "m".into(),
                    sessions: 1,
                    pins: Arc::new(Shelf::default()),
                    integration_branch: "main_agent".into(),
                })),
                data: None,
                plant: None,
                pins: None,
            })
        };
        let left = make();
        left.draw_mockups().await.expect("started");
        assert!(left.draw_mockups().await.is_err(), "one job at a time");
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(left.job().map(|j| j.state), Some(State::Done));
        let stopped = make();
        stopped.draw_mockups().await.expect("started");
        tokio::time::sleep(Duration::from_millis(30)).await;
        stopped.shutdown();
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(
            stopped.job().map(|j| j.state),
            Some(State::Running),
            "aborted, the job never ends"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_micro_frontend_without_its_fragment_stops_the_initialization_before_anything_is_published()
     {
        let dir = product("inline");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            scope: "## Goal\nproof".to_string(),
            mocked: Some("drawn".to_string()),
            questions: vec![Question {
                id: 1,
                title: "who".to_string(),
                answer: "the client".to_string(),
                ..Question::default()
            }],
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: Some(Arc::new(Drawer { lazy: true })),
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: Some(Arc::clone(&mockups)),
            kit: Some(KitDesk::new(kit_desk::Setup {
                mockups,
                oneshot: None,
                model: "m".into(),
                sessions: 1,
                pins: Arc::new(Shelf::default()),
                integration_branch: "main_agent".into(),
            })),
            data: None,
            plant: None,
            pins: None,
        });
        let kit = desk.kit().await.expect("a kit");
        assert!(kit.validated);
        assert_eq!((kit.waiting, kit.generated, kit.running), (0, 0, false));
        rules_validated(&desk);
        desk.accept().await.expect("accepted");
        let job = settled(&desk).await;
        assert_eq!(job.state, State::Failed);
        assert_eq!(
            job.why,
            "the mock-up is not made of micro-frontends yet: `Home` has no fragment"
        );
        assert!(
            !job.steps.iter().any(|s| s.starts_with("publishing")),
            "{:?}",
            job.steps
        );
        assert_eq!(
            desk.record()
                .expect("read")
                .expect("a record")
                .componentized,
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session that draws two pages, several seconds apart — slow enough
    /// that a poller's own 2s tick lands on each page in turn, not just the
    /// end of a fast call.
    struct StagedPages;

    impl Oneshot for StagedPages {
        fn ask(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }

        fn work_in(&self, _: &str, _: &str, dir: &Path) -> Result<String, String> {
            let write = |name: &str, manifest: &str| -> Result<(), String> {
                std::fs::create_dir_all(dir.join("mock/pages")).map_err(|e| e.to_string())?;
                std::fs::write(dir.join(format!("mock/pages/{name}.html")), "<main></main>")
                    .map_err(|e| e.to_string())?;
                std::fs::write(dir.join("mock/pages.json"), manifest).map_err(|e| e.to_string())
            };
            // "C" is listed from the start but never gets a file: it must
            // never be reported, whatever a mutated `||` would let through.
            write(
                "a",
                r#"{"version":1,"pages":[{"path":"pages/a.html","title":"A"},{"path":"pages/c.html","title":"C"}],"components":[]}"#,
            )?;
            std::thread::sleep(DRAW_POLL + Duration::from_secs(1));
            write(
                "b",
                r#"{"version":1,"pages":[{"path":"pages/a.html","title":"A"},{"path":"pages/b.html","title":"B"},{"path":"pages/c.html","title":"C"}],"components":[]}"#,
            )?;
            // Left on disk long enough for the poller's next tick to see it
            // before the call returns and the poller is aborted.
            std::thread::sleep(DRAW_POLL + Duration::from_secs(1));
            Ok("two pages".to_string())
        }
    }

    #[tokio::test]
    async fn drawing_the_mock_up_reports_each_page_live_as_it_is_written() {
        let dir = product("staged-pages");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let kit = KitDesk::new(kit_desk::Setup {
            mockups: Arc::clone(&mockups),
            oneshot: None,
            model: "m".into(),
            sessions: 1,
            pins: Arc::new(Shelf::default()),
            integration_branch: "main_agent".into(),
        });
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            questions: vec![Question {
                id: 1,
                title: "who".to_string(),
                answer: "the client".to_string(),
                ..Question::default()
            }],
            scope: "## Goal\nproof".to_string(),
            step: Step::Mockups,
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: Some(Arc::new(StagedPages)),
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: Some(mockups),
            kit: Some(kit),
            data: None,
            plant: None,
            pins: None,
        });
        desk.draw_mockups().await.expect("started");
        // The first page's own line shows up while the session is still
        // mid-write, well before `work_in` itself returns.
        let mut seen_a = false;
        for _ in 0..80 {
            if desk
                .job()
                .is_some_and(|j| j.steps.iter().any(|s| s.contains("\"A\" page (1 of 2)")))
            {
                seen_a = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(seen_a, "the first page's step never showed up live");
        assert_eq!(
            desk.job().map(|j| j.state),
            Some(State::Running),
            "the session itself had not returned yet"
        );
        let job = settled(&desk).await;
        assert_eq!((job.work, job.state), (Work::Mockups, State::Done));
        assert!(
            job.steps.iter().any(|s| s.contains("\"B\" page (2 of 3)")),
            "{:?}",
            job.steps
        );
        assert!(
            !job.steps.iter().any(|s| s.contains("\"C\" page")),
            "listed with no file ever written: never reported done — {:?}",
            job.steps
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session that leaves an already-made component as it is and makes
    /// one brand new one, several seconds in.
    struct StagedComponentize;

    impl Oneshot for StagedComponentize {
        fn ask(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }

        fn work_in(&self, _: &str, prompt: &str, dir: &Path) -> Result<String, String> {
            assert!(
                prompt.contains("Make it again of **micro-frontends**"),
                "the right prompt"
            );
            std::thread::sleep(DRAW_POLL + Duration::from_secs(1));
            let write = |path: &str, text: &str| -> Result<(), String> {
                let full = dir.join(path);
                std::fs::create_dir_all(full.parent().ok_or("no folder")?)
                    .map_err(|e| e.to_string())?;
                std::fs::write(full, text).map_err(|e| e.to_string())
            };
            write("mock/components/New.html", "<section>new</section>")?;
            write(
                "mock/pages.json",
                r#"{"version":1,"pages":[{"path":"pages/home.html","title":"Home","components":["Old","New"]}],"components":[{"name":"Old","fragment":"components/Old.html"},{"name":"New","fragment":"components/New.html"}]}"#,
            )?;
            write(
                "mock/pages/home.html",
                r#"<div data-component="Old"></div><div data-component="New"></div>"#,
            )?;
            // Left on disk long enough for the poller's next tick to see it
            // before the call returns and the poller is aborted.
            std::thread::sleep(DRAW_POLL + Duration::from_secs(1));
            Ok("one new micro-frontend".to_string())
        }
    }

    #[tokio::test]
    async fn remaking_micro_frontends_never_reports_one_already_there_as_just_made() {
        let dir = product("staged-componentize");
        // An already-made component, as a prior attempt would have left it.
        std::fs::write(
            dir.join("main_agent/mock/pages.json"),
            r#"{"version":1,"pages":[{"path":"pages/home.html","title":"Home","components":["Old"]}],"components":[{"name":"Old","fragment":"components/Old.html"}]}"#,
        )
        .expect("seeded");
        std::fs::create_dir_all(dir.join("main_agent/mock/components")).expect("dir");
        std::fs::write(
            dir.join("main_agent/mock/components/Old.html"),
            "<section>old</section>",
        )
        .expect("seeded");
        std::fs::create_dir_all(dir.join("main_agent/mock/pages")).expect("dir");
        std::fs::write(
            dir.join("main_agent/mock/pages/home.html"),
            r#"<div data-component="Old"></div>"#,
        )
        .expect("seeded");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let kit = KitDesk::new(kit_desk::Setup {
            mockups: Arc::clone(&mockups),
            oneshot: None,
            model: "m".into(),
            sessions: 1,
            pins: Arc::new(Shelf::default()),
            integration_branch: "main_agent".into(),
        });
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            questions: vec![Question {
                id: 1,
                title: "who".to_string(),
                answer: "the client".to_string(),
                ..Question::default()
            }],
            scope: "## Goal\nproof".to_string(),
            mocked: Some("drawn".to_string()),
            ..Creation::new("alpha", "me/alpha")
        };
        book.keep_creation("alpha", &serde_json::to_string(&record).expect("json"))
            .expect("kept");
        let desk = CreationDesk::new(Setup {
            register: book,
            factory: "alpha".to_string(),
            foundry: Arc::new(Nothing),
            factories_dir: dir.join("factories"),
            integration_branch: "main_agent".to_string(),
            oneshot: Some(Arc::new(StagedComponentize)),
            model: "m".to_string(),
            draw_model: "d".to_string(),
            mockups: Some(mockups),
            kit: Some(kit),
            data: None,
            plant: None,
            pins: None,
        });
        rules_validated(&desk);
        desk.accept().await.expect("accepted");
        let mut seen_new = false;
        for _ in 0..80 {
            if desk
                .job()
                .is_some_and(|j| j.steps.iter().any(|s| s.contains("\"New\" micro-frontend")))
            {
                seen_new = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(seen_new, "the new component's step never showed up live");
        let job = settled(&desk).await;
        assert!(
            !job.steps
                .iter()
                .any(|s| s.contains("\"Old\" micro-frontend")),
            "an already-made component must never be reported as just made: {:?}",
            job.steps
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
