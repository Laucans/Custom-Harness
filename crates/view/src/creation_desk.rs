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
use crate::domain::creation::{
    self, ComponentRef, Context, Creation, DesignRef, Edit, Facts, PageRef, PinRef, Step,
    grill_prompt, injected, not_componentized, parse_requirements, parse_round,
    requirements_prompt, scope_prompt, unfenced,
};
use crate::domain::design::Entry;
use crate::domain::divergence;
use crate::domain::drawing::{
    Cut, Planned, Reference, cut_prompt, index_html, parse_cut, planned, reference_prompt,
    screen_prompt, screens, screens_changed, to_cut, with_cut, with_pages,
};
use crate::domain::note_session::{About, mockup_rules};
use crate::domain::poc_model::{Drafts, Gaps, drafts_prompt, gaps, parse_drafts};
use crate::kit_desk::{JobState, KitDesk};
use crate::ports::{Foundry, Mockups, NoteStore, Oneshot, Plant};
use harness_architecture::kit::{ComponentState, DesignState};
use harness_core::domain::Slug;
use harness_core::ports::store::factories::{Factories, id_from};

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
    /// How many sessions draw or cut pages at once.
    pub sessions: usize,
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
    /// grilling: the design system's tokens (the kit follows), a first
    /// scope.
    Prepare,
    /// The first page, drawn while the scope is reviewed: the look every
    /// other page follows.
    Reference,
    /// The proof of concept drawn as mock-ups: every other page at once,
    /// and the data and the ports drafted beside them.
    Mockups,
    /// The mock-up made again of micro-frontends, page by page: those not
    /// cut yet, or moved since.
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
        }
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
        self.setup
            .register
            .keep_creation(&self.setup.factory, &text)
            .map_err(|why| why.reason().to_string())
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
    /// no tokens yet — the ticked kit components queue behind them — then a
    /// first scope, proposed from the brief and the decisions taken so far
    /// unless the human wrote one already. Nothing here closes a gate: the
    /// scope is proposed again once the grilling moved the decisions.
    ///
    /// # Errors
    ///
    /// No creation, no brief, or another job under way.
    pub async fn prepare(&self) -> Result<(), String> {
        let record = self.existing()?;
        record.may_enter(Step::Grilling, self.facts().await)?;
        self.begin(Work::Prepare)?;
        let desk = self.clone();
        self.spawn(async move { desk.prepared(record).await });
        Ok(())
    }

    async fn prepared(&self, record: Creation) -> Result<(), String> {
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
        if record.scope_edited || !record.scope.trim().is_empty() && !record.scope_stale() {
            self.step("the scope is written already");
            return Ok(());
        }
        if self.setup.oneshot.is_none() {
            self.step(NO_CLAUDE);
            return Ok(());
        }
        self.step("proposing a first scope from the brief");
        let scope = unfenced(&self.ask(scope_prompt(&record)).await?);
        let _held = self.record.lock().await;
        let mut kept = self.existing()?;
        if kept.scope_edited {
            self.step("the human wrote the scope meanwhile: the proposal is dropped");
            return Ok(());
        }
        kept.propose(scope);
        // The fingerprint is the brief's as it was when the job started, so
        // decisions taken meanwhile show the scope as stale, as they should.
        kept.scope_from = record.fingerprint();
        self.keep(&kept)?;
        self.step(
            "a first scope is proposed; it is proposed again once the grilling moved the decisions",
        );
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
        let asked = self.existing()?;
        asked.may_enter(Step::Review, self.facts().await)?;
        let ctx = self.context(&asked).await;
        let pages: Vec<String> = ctx.pages.iter().map(|p| p.path.clone()).collect();
        let components: Vec<String> = ctx.components.iter().map(|c| c.name.clone()).collect();
        let answer = self.ask(requirements_prompt(&asked, &ctx)).await?;
        let read = parse_requirements(&answer, &pages, &components)?;
        let _held = self.record.lock().await;
        let mut record = self.existing()?;
        record.add_requirements(read);
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
        });
        Ok(())
    }

    fn update(&self, change: impl FnOnce(&mut Job)) {
        if let Some(job) = lock(&self.job).as_mut() {
            change(job);
        }
    }

    fn step(&self, line: &str) {
        let line = line.to_string();
        self.update(|job| job.steps.push(line));
    }

    fn end(&self, outcome: Result<(), String>) {
        self.update(|job| match outcome {
            Ok(()) => job.state = State::Done,
            Err(why) => {
                job.state = State::Failed;
                job.why = why;
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

    /// The kit's components, one line each, as the drawing prompts list them.
    async fn kit_lines(&self) -> String {
        match &self.setup.kit {
            Some(kit) => kit.index().await.map(|i| i.lines()).unwrap_or_default(),
            None => String::new(),
        }
    }

    fn rules(&self) -> String {
        mockup_rules(&About {
            branch: self.setup.integration_branch.clone(),
            ..About::default()
        })
    }

    /// Draws the reference page while the human reviews the scope: the
    /// first screen of the proposed scope, on the design system — the look
    /// every other page then follows.
    ///
    /// # Errors
    ///
    /// The grilling is not over, no scope yet, the design system not ready,
    /// a reference drawn already, nothing can draw, or another job under way.
    pub async fn draw_reference(&self) -> Result<(), String> {
        let record = self.existing()?;
        record.may_draw_reference(self.facts().await)?;
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let mockups = self.setup.mockups.clone().ok_or(NO_PRODUCT)?;
        self.begin(Work::Reference)?;
        let desk = self.clone();
        self.spawn(async move { desk.referenced(record, oneshot, mockups).await });
        Ok(())
    }

    async fn referenced(
        &self,
        record: Creation,
        oneshot: Arc<dyn Oneshot>,
        mockups: Arc<dyn Mockups>,
    ) -> Result<(), String> {
        let now = screens(&record.scope);
        let all = planned(&now);
        let first = all
            .first()
            .cloned()
            .ok_or("the scope lists no screen under `## Screens`: nothing to draw")?;
        let branch = self.setup.integration_branch.clone();
        let prompt = reference_prompt(
            &record,
            &self.rules(),
            &self.kit_lines().await,
            &first,
            &all,
            false,
        );
        self.step("opening the integration branch's mock-up");
        let fresh = mockups.clone();
        let at = branch.clone();
        tokio::task::spawn_blocking(move || {
            // A drawing starts from the branch as committed.
            let _ = fresh.discard(&at);
            fresh.workspace(&at)
        })
        .await
        .map_err(|e| format!("the step did not finish: {e}"))??;
        self.step(&format!(
            "a Claude Code session draws the reference page, \"{}\"",
            first.screen.name
        ));
        let said = self
            .draw_pages(oneshot, mockups.clone(), vec![(first.clone(), prompt)])
            .await?;
        list_pages(
            mockups,
            &branch,
            &record.name,
            std::slice::from_ref(&first),
            &[],
        )
        .await?;
        self.step(&format!("the reference page is drawn: {said}"));
        let _held = self.record.lock().await;
        let mut kept = self.existing()?;
        kept.reference = Some(Reference {
            screen: first.screen.name.clone(),
            path: first.path.clone(),
            screens: now,
        });
        self.keep(&kept)
    }

    /// Runs one drawing session per page, `sessions` at a time, reporting
    /// each page as its file lands; answers what the sessions said, one
    /// line a page.
    async fn draw_pages(
        &self,
        oneshot: Arc<dyn Oneshot>,
        mockups: Arc<dyn Mockups>,
        pages: Vec<(Planned, String)>,
    ) -> Result<String, String> {
        let branch = self.setup.integration_branch.clone();
        let model = self.setup.model.clone();
        let watched: Vec<(String, String)> = pages
            .iter()
            .map(|(p, _)| (p.path.clone(), p.screen.name.clone()))
            .collect();
        let poll = tokio::spawn(poll_pages(
            self.clone(),
            mockups.clone(),
            branch.clone(),
            watched,
        ));
        let dir_of = mockups.clone();
        let at = branch.clone();
        let dir = tokio::task::spawn_blocking(move || dir_of.workspace(&at))
            .await
            .map_err(|e| format!("the step did not finish: {e}"))??;
        let jobs: Vec<_> = pages
            .into_iter()
            .map(|(page, prompt)| {
                let (oneshot, model, dir) = (Arc::clone(&oneshot), model.clone(), dir.clone());
                let (mockups, branch) = (Arc::clone(&mockups), branch.clone());
                move || {
                    let said = oneshot
                        .work_in(&model, &prompt, &dir)
                        .map_err(|why| format!("{}: {why}", page.screen.name))?;
                    let drawn = mockups
                        .file(&branch, &page.path)
                        .map_err(|why| format!("{}: {why}", page.screen.name))?
                        .is_some_and(|bytes| !bytes.is_empty());
                    if !drawn {
                        return Err(format!(
                            "{}: the session wrote no `mock/{}`",
                            page.screen.name, page.path
                        ));
                    }
                    Ok(format!("{}: {}", page.screen.name, said.trim()))
                }
            })
            .collect();
        let outcomes = in_parallel(self.setup.sessions, jobs).await;
        poll.abort();
        let (said, failed): (Vec<_>, Vec<_>) = outcomes.into_iter().partition(Result::is_ok);
        let failed: Vec<String> = failed.into_iter().filter_map(Result::err).collect();
        if !failed.is_empty() {
            return Err(format!("a page was not drawn — {}", failed.join("; ")));
        }
        Ok(said
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Draws the proof of concept as the product's mock-up, on the
    /// integration branch's workspace: every page at once, on the
    /// reference's model, then the review.
    ///
    /// # Errors
    ///
    /// The step's gate is closed, nothing can draw, or another job is under
    /// way.
    pub async fn draw_mockups(&self) -> Result<(), String> {
        let mut record = self.existing()?;
        record.may_enter(Step::Mockups, self.facts().await)?;
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let mockups = self.setup.mockups.clone().ok_or(NO_PRODUCT)?;
        self.begin(Work::Mockups)?;
        if record.mocked.is_some() {
            // Drawn again: everything goes, the reference too.
            let _held = self.record.lock().await;
            let mut kept = self.existing()?;
            kept.reference = None;
            kept.mocked = None;
            kept.componentized = None;
            kept.cut.clear();
            self.keep(&kept)?;
            record = kept;
        }
        let desk = self.clone();
        self.spawn(async move { desk.drawn(record, oneshot, mockups).await });
        Ok(())
    }

    // One job, read top to bottom: the reference, the other pages, the drafts.
    #[allow(clippy::too_many_lines)]
    async fn drawn(
        &self,
        record: Creation,
        oneshot: Arc<dyn Oneshot>,
        mockups: Arc<dyn Mockups>,
    ) -> Result<(), String> {
        let branch = self.setup.integration_branch.clone();
        let now = screens(&record.scope);
        let mut all = planned(&now);
        if all.is_empty() {
            return Err(
                "the scope lists no screen under `## Screens`: nothing to draw".to_string(),
            );
        }
        let (rules, kit) = (self.rules(), self.kit_lines().await);
        let there = match &record.reference {
            Some(r) => {
                let (m, at, path) = (mockups.clone(), branch.clone(), r.path.clone());
                tokio::task::spawn_blocking(move || m.file(&at, &path))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten()
                    .is_some_and(|bytes| !bytes.is_empty())
            }
            None => false,
        };
        let mut drop: Vec<String> = Vec::new();
        let mut reference: Option<Reference> = record.reference.clone().filter(|_| there);
        // What the reference's own session said, when it drew this time.
        let mut first_said = String::new();
        let model = match &reference {
            None => {
                self.step("opening the integration branch's mock-up");
                let (m, at) = (mockups.clone(), branch.clone());
                tokio::task::spawn_blocking(move || {
                    let _ = m.discard(&at);
                    m.workspace(&at)
                })
                .await
                .map_err(|e| format!("the step did not finish: {e}"))??;
                let first = all[0].clone();
                self.step(&format!(
                    "a Claude Code session draws the reference page, \"{}\"",
                    first.screen.name
                ));
                let prompt = reference_prompt(&record, &rules, &kit, &first, &all, false);
                first_said = self
                    .draw_pages(
                        oneshot.clone(),
                        mockups.clone(),
                        vec![(first.clone(), prompt)],
                    )
                    .await?;
                reference = Some(Reference {
                    screen: first.screen.name.clone(),
                    path: first.path.clone(),
                    screens: now.clone(),
                });
                first.path
            }
            Some(r) => {
                let changes = screens_changed(&r.screens, &now);
                let same = |name: &str| name.to_lowercase() == r.screen.to_lowercase();
                if let Some(i) = all.iter().position(|p| same(&p.screen.name)) {
                    // Its page stays where it was drawn.
                    if !all.iter().any(|p| p.path == r.path) {
                        all[i].path.clone_from(&r.path);
                    }
                    let page = all[i].clone();
                    if changes.changed.iter().any(|n| same(n)) {
                        self.step(&format!(
                                "the scope changed \"{}\": the reference page is drawn again, on its own look",
                                page.screen.name
                            ));
                        let prompt = reference_prompt(&record, &rules, &kit, &page, &all, true);
                        first_said = self
                            .draw_pages(
                                oneshot.clone(),
                                mockups.clone(),
                                vec![(page.clone(), prompt)],
                            )
                            .await?;
                    } else {
                        self.step(&format!(
                            "the reference page \"{}\" is kept: the scope did not change it",
                            page.screen.name
                        ));
                    }
                    page.path
                } else {
                    self.step(&format!(
                        "the scope dropped \"{}\": its page lends its look, then goes",
                        r.screen
                    ));
                    drop.push(r.path.clone());
                    r.path.clone()
                }
            }
        };
        let others: Vec<(Planned, String)> = all
            .iter()
            .filter(|p| p.path != model)
            .map(|p| {
                (
                    p.clone(),
                    screen_prompt(&record, &rules, &kit, p, &model, &all),
                )
            })
            .collect();
        self.step(&format!(
            "{} Claude Code session(s) draw the other pages, {} at a time, while the data and the ports are drafted",
            others.len(),
            self.setup.sessions.max(1)
        ));
        let drafting = {
            let (oneshot, model, prompt) = (
                Arc::clone(&oneshot),
                self.setup.model.clone(),
                drafts_prompt(&record),
            );
            tokio::task::spawn_blocking(move || oneshot.ask(&model, &prompt))
        };
        let drawn = self.draw_pages(oneshot, mockups.clone(), others).await;
        let drafts = match drafting.await {
            Ok(Ok(answer)) => match parse_drafts(&answer) {
                Ok(drafts) => {
                    self.step(&format!(
                        "drafted {} table(s) and {} port(s) from the scope",
                        drafts.tables().len(),
                        drafts.ports.len()
                    ));
                    Some(drafts)
                }
                Err(why) => {
                    self.step(&format!("the drafts are left out: {why}"));
                    None
                }
            },
            Ok(Err(why)) => {
                self.step(&format!("the drafts are left out: {why}"));
                None
            }
            Err(e) => {
                self.step(&format!("the drafts are left out: {e}"));
                None
            }
        };
        let said = [first_said, drawn?]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        for path in &drop {
            let (m, at, path) = (mockups.clone(), branch.clone(), path.clone());
            tokio::task::spawn_blocking(move || m.remove(&at, &path))
                .await
                .map_err(|e| format!("the step did not finish: {e}"))??;
        }
        list_pages(mockups, &branch, &record.name, &all, &drop).await?;
        self.step("the mock-up is drawn");
        let facts = self.facts().await;
        let _held = self.record.lock().await;
        let mut kept = self.existing()?;
        kept.reference = reference;
        kept.drafts = drafts.or(kept.drafts);
        kept.mocked = Some(said);
        kept.go(Step::Review, facts)?;
        self.keep(&kept)
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
        // The job first: an acceptance nothing starts is not kept.
        self.begin(Work::Initialize)?;
        self.keep(&record)?;
        let desk = self.clone();
        let kept = record.clone();
        self.spawn(async move { desk.initialized(kept).await });
        Ok(record)
    }

    /// The mock-up made again of micro-frontends, as its own job — before
    /// the review, so the requirements can be tied to them: the pages not
    /// cut yet, or whose file moved since. None of them: nothing runs.
    ///
    /// # Errors
    ///
    /// No mock-up drawn, nothing can write, or another job under way.
    pub async fn componentize_now(&self) -> Result<(), String> {
        let record = self.existing()?;
        record.may_enter(Step::Review, self.facts().await)?;
        if record.componentized.is_some() && !self.cut_stale(&record).await {
            return Ok(());
        }
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let mockups = self.setup.mockups.clone().ok_or(NO_PRODUCT)?;
        self.begin(Work::Componentize)?;
        let desk = self.clone();
        self.spawn(async move { desk.componentize(&record, oneshot, mockups).await });
        Ok(())
    }

    /// The pages of the mock-up and their fingerprints, as they stand.
    async fn page_hashes(&self) -> Vec<(String, String)> {
        let Some(mockups) = self.setup.mockups.clone() else {
            return Vec::new();
        };
        let branch = self.setup.integration_branch.clone();
        tokio::task::spawn_blocking(move || mockups.state(&branch))
            .await
            .ok()
            .and_then(Result::ok)
            .map(|state| state.pages.into_iter().map(|p| (p.path, p.hash)).collect())
            .unwrap_or_default()
    }

    /// A page moved since it was cut, or was never cut.
    pub async fn cut_stale(&self, record: &Creation) -> bool {
        record.componentized.is_some() && !to_cut(&self.page_hashes().await, &record.cut).is_empty()
    }

    /// Where the drafted ports and the micro-frontends disagree; `None`
    /// before anything is drafted.
    pub async fn gaps(&self, record: &Creation) -> Option<Gaps> {
        let drafts: &Drafts = record.drafts.as_ref()?;
        let ctx = self.context(record).await;
        Some(gaps(&drafts.ports, &ctx.components))
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
        let at = branch.clone();
        let wrote = mockups.clone();
        tokio::task::spawn_blocking(move || {
            wrote.write(&at, creation::RECORD_JSON, json.as_bytes())?;
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

    /// Makes the mock-up again of micro-frontends, page by page, in the
    /// integration branch's workspace: the pages not cut yet or moved since
    /// — the first page alone when nothing is cut yet, so the parts every
    /// page shares are made once — then checks every page is made of listed
    /// ones: what the micro-frontends' view and the code stage read.
    // One job, read top to bottom: the batches, the merge, the gate.
    #[allow(clippy::too_many_lines)]
    async fn componentize(
        &self,
        record: &Creation,
        oneshot: Arc<dyn Oneshot>,
        mockups: Arc<dyn Mockups>,
    ) -> Result<(), String> {
        let branch = self.setup.integration_branch.clone();
        let (rules, kit) = (self.rules(), self.kit_lines().await);
        let (m, at) = (mockups.clone(), branch.clone());
        let state = tokio::task::spawn_blocking(move || m.state(&at))
            .await
            .map_err(|e| format!("the step did not finish: {e}"))??;
        let hashes: Vec<(String, String)> = state
            .pages
            .iter()
            .map(|p| (p.path.clone(), p.hash.clone()))
            .collect();
        let todo = to_cut(&hashes, &record.cut);
        // Seeded with whatever is already there: a retry's own earlier
        // micro-frontends are never reported as newly made.
        let mut seen: HashSet<String> = state.components.iter().map(|c| c.name.clone()).collect();
        let mut batches: Vec<Vec<String>> = Vec::new();
        match todo.split_first() {
            Some((first, rest)) if state.components.is_empty() && !rest.is_empty() => {
                batches.push(vec![first.clone()]);
                batches.push(rest.to_vec());
            }
            _ => batches.push(todo.clone()),
        }
        let mut said: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        for batch in batches.into_iter().filter(|b| !b.is_empty()) {
            self.step(&format!(
                "{} Claude Code session(s) make {} again of micro-frontends",
                batch.len(),
                batch.join(", ")
            ));
            let (m, at) = (mockups.clone(), branch.clone());
            let existing = tokio::task::spawn_blocking(move || m.state(&at))
                .await
                .ok()
                .and_then(Result::ok)
                .map(|state| {
                    state
                        .components
                        .iter()
                        .map(|c| format!("- {} ({}) — {}", c.name, c.system, c.description))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let (m, at) = (mockups.clone(), branch.clone());
            let dir = tokio::task::spawn_blocking(move || m.workspace(&at))
                .await
                .map_err(|e| format!("the step did not finish: {e}"))??;
            let jobs: Vec<_> = batch
                .iter()
                .map(|page| {
                    let prompt = cut_prompt(record, &rules, &kit, page, &existing);
                    let (oneshot, model, dir, page) = (
                        Arc::clone(&oneshot),
                        self.setup.model.clone(),
                        dir.clone(),
                        page.clone(),
                    );
                    move || {
                        oneshot
                            .work_in(&model, &prompt, &dir)
                            .map(|said| (page.clone(), said))
                            .map_err(|why| format!("{page}: {why}"))
                    }
                })
                .collect();
            for outcome in in_parallel(self.setup.sessions, jobs).await {
                match outcome {
                    Ok((page, answer)) => {
                        let (m, at, p) = (mockups.clone(), branch.clone(), page.clone());
                        let listed = tokio::task::spawn_blocking(move || {
                            let html = m.file(&at, &p)?.unwrap_or_default();
                            let names = injected(&String::from_utf8_lossy(&html));
                            let pages = m
                                .file(&at, "pages.json")?
                                .map(|b| String::from_utf8_lossy(&b).into_owned());
                            let text = with_cut(pages.as_deref(), &p, &names, &parse_cut(&answer))?;
                            m.write(&at, "pages.json", text.as_bytes())?;
                            Ok::<_, String>(names.len())
                        })
                        .await
                        .map_err(|e| format!("the step did not finish: {e}"))?;
                        match listed {
                            Ok(n) => said.push(format!("{page}: {n} micro-frontend(s)")),
                            Err(why) => failed.push(format!("{page}: {why}")),
                        }
                        self.report_made(&mockups, &mut seen).await;
                    }
                    Err(why) => failed.push(why),
                }
            }
        }
        if !failed.is_empty() {
            return Err(format!(
                "a page was not made of micro-frontends — {}",
                failed.join("; ")
            ));
        }
        let (m, at) = (mockups.clone(), branch.clone());
        let (problems, cut) = tokio::task::spawn_blocking(move || {
            let state = m.state(&at)?;
            let mut pages = Vec::new();
            for page in &state.pages {
                let html = m.file(&at, &page.path)?.unwrap_or_default();
                pages.push((
                    page.path.clone(),
                    String::from_utf8_lossy(&html).into_owned(),
                ));
            }
            let mut components = Vec::new();
            for c in &state.components {
                // An empty path is no file: the port reads nothing there.
                let there = m.file(&at, &c.fragment)?.is_some();
                components.push((c.name.clone(), there));
            }
            let cut: Vec<Cut> = state
                .pages
                .into_iter()
                .map(|p| Cut {
                    path: p.path,
                    hash: p.hash,
                })
                .collect();
            Ok::<_, String>((not_componentized(&pages, &components), cut))
        })
        .await
        .map_err(|e| format!("the step did not finish: {e}"))??;
        if !problems.is_empty() {
            return Err(format!(
                "the mock-up is not made of micro-frontends yet: {}",
                problems.join("; ")
            ));
        }
        self.step("every page is made of micro-frontends");
        let _held = self.record.lock().await;
        let mut kept = self.existing()?;
        let summary = if said.is_empty() {
            "every page was cut already".to_string()
        } else {
            said.join("\n")
        };
        kept.componentized = Some(match kept.componentized.take() {
            Some(before) if said.is_empty() => before,
            _ => summary,
        });
        kept.cut = cut;
        self.keep(&kept)
    }

    /// Reports each micro-frontend listed with its fragment that `seen` has
    /// not reported yet.
    async fn report_made(&self, mockups: &Arc<dyn Mockups>, seen: &mut HashSet<String>) {
        let (m, at) = (Arc::clone(mockups), self.setup.integration_branch.clone());
        let Ok(Ok((total, built))) = tokio::task::spawn_blocking(move || {
            let state = m.state(&at)?;
            let mut built = Vec::new();
            for c in &state.components {
                built.push((c.name.clone(), m.file(&at, &c.fragment)?.is_some()));
            }
            Ok::<_, String>((state.components.len(), built))
        })
        .await
        else {
            return;
        };
        for (name, there) in built {
            if !there || seen.contains(&name) {
                continue;
            }
            seen.insert(name.clone());
            self.step(&format!(
                "made the \"{name}\" micro-frontend real ({} of {total})",
                seen.len()
            ));
        }
    }

    /// Stops what is under way.
    pub fn shutdown(&self) {
        let task = lock(&self.task).take();
        if let Some(task) = task {
            task.abort();
        }
    }
}

/// How often the mock-up's drawing and the micro-frontends' remake are
/// polled for a page or a component that just landed.
const DRAW_POLL: Duration = Duration::from_secs(2);

/// Reports each page of `pages` (path, title) as its file lands in
/// `branch`'s workspace. Runs until aborted — the drawing stops it once its
/// sessions returned; a read that fails is skipped, never a reason the job
/// failed, since the real outcome is the sessions' own.
async fn poll_pages(
    desk: CreationDesk,
    mockups: Arc<dyn Mockups>,
    branch: String,
    pages: Vec<(String, String)>,
) {
    let mut seen: HashSet<String> = HashSet::new();
    let total = pages.len();
    loop {
        tokio::time::sleep(DRAW_POLL).await;
        for (path, title) in &pages {
            if seen.contains(path) {
                continue;
            }
            let (m, at, p) = (mockups.clone(), branch.clone(), path.clone());
            let Ok(Ok(Some(bytes))) = tokio::task::spawn_blocking(move || m.file(&at, &p)).await
            else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            seen.insert(path.clone());
            desk.step(&format!(
                "drew the mock-up's \"{title}\" page ({} of {total})",
                seen.len()
            ));
        }
    }
}

/// Runs `jobs` on blocking threads, at most `limit` at a time, and answers
/// their outcomes in the order given.
async fn in_parallel<T, F>(limit: usize, jobs: Vec<F>) -> Vec<Result<T, String>>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let gate = Arc::new(tokio::sync::Semaphore::new(limit.max(1)));
    let mut set = tokio::task::JoinSet::new();
    let count = jobs.len();
    for (i, job) in jobs.into_iter().enumerate() {
        let permit = Arc::clone(&gate).acquire_owned().await;
        set.spawn_blocking(move || {
            let outcome = job();
            drop(permit);
            (i, outcome)
        });
    }
    let mut outcomes: Vec<Option<Result<T, String>>> = (0..count).map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((i, outcome)) => {
                if let Some(slot) = outcomes.get_mut(i) {
                    *slot = Some(outcome);
                }
            }
            Err(e) => {
                if let Some(slot) = outcomes.iter_mut().find(|o| o.is_none()) {
                    *slot = Some(Err(format!("a session did not finish: {e}")));
                }
            }
        }
    }
    outcomes
        .into_iter()
        .map(|o| o.unwrap_or_else(|| Err("a session did not finish".to_string())))
        .collect()
}

/// The pages listed in `branch`'s `mock/pages.json` — those of `drop` gone
/// — and its `mock/index.html` written, by the desk: the sessions drawing
/// in parallel never write either.
async fn list_pages(
    mockups: Arc<dyn Mockups>,
    branch: &str,
    name: &str,
    pages: &[Planned],
    drop: &[String],
) -> Result<(), String> {
    let (branch, name, pages, drop) = (
        branch.to_string(),
        name.to_string(),
        pages.to_vec(),
        drop.to_vec(),
    );
    tokio::task::spawn_blocking(move || {
        let current = mockups
            .file(&branch, "pages.json")?
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let listed = with_pages(current.as_deref(), &pages, &drop)?;
        mockups.write(&branch, "pages.json", listed.as_bytes())?;
        let state = mockups.state(&branch)?;
        let all: Vec<Planned> = state
            .pages
            .iter()
            .filter_map(|p| pages.iter().find(|q| q.path == p.path).cloned())
            .collect();
        mockups.write(&branch, "index.html", index_html(&name, &all).as_bytes())
    })
    .await
    .map_err(|e| format!("the step did not finish: {e}"))?
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

    /// The page a drawing prompt asks for, relative to the folder.
    fn asked_page(prompt: &str) -> Option<String> {
        let rest = prompt.split("Write `mock/").nth(1)?;
        Some(format!("mock/{}", rest.split('`').next()?))
    }

    /// The page a cutting prompt asks for, relative to the folder.
    fn cut_page(prompt: &str) -> Option<String> {
        let rest = prompt.split("Make the page `mock/").nth(1)?;
        Some(format!("mock/{}", rest.split('`').next()?))
    }

    /// A session that draws the page it is asked for, then cuts it into
    /// one micro-frontend — or, `lazy`, injects it and never writes its
    /// fragment. Every prompt it is put to work with is kept.
    #[derive(Default)]
    struct Drawer {
        lazy: bool,
        prompts: Mutex<Vec<String>>,
    }

    impl Drawer {
        fn new(lazy: bool) -> Self {
            Self {
                lazy,
                ..Self::default()
            }
        }

        fn prompts(&self) -> Vec<String> {
            lock(&self.prompts).clone()
        }
    }

    impl Oneshot for Drawer {
        fn ask(&self, _: &str, prompt: &str) -> Result<String, String> {
            if prompt.contains("read the requirements") {
                return Ok(r#"{"requirements":[{"title":"See the ledger","body":"the lines","pages":["pages/home.html"],"components":["Home","Ghost"]}]}"#.to_string());
            }
            if prompt.contains("You draft the foundations") {
                return Ok(r#"{"model":{"version":1,"tables":[{"name":"line"}]},"ports":[{"id":"ledger.listLines","kind":"capability"},{"id":"ledger.addLine","kind":"data_capability"}]}"#.to_string());
            }
            Ok("## Goal\nfirst".to_string())
        }

        fn work_in(&self, _: &str, prompt: &str, dir: &Path) -> Result<String, String> {
            lock(&self.prompts).push(prompt.to_string());
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
            assert!(prompt.contains("a ledger"), "the brief is in the prompt");
            if let Some(page) = cut_page(prompt) {
                if !self.lazy {
                    write("mock/components/Home.html", "<section>home</section>")?;
                }
                write(&page, r#"<div data-component="Home"></div>"#)?;
                return Ok(r#"one micro-frontend: Home {"components":[{"name":"Home","system":"ledger","description":"the ledger","needs":["ledger.listLines"],"actions":[]}]}"#.to_string());
            }
            assert!(prompt.contains("## Screens"), "the scope is in the prompt");
            let page = asked_page(prompt).ok_or("no page asked")?;
            write(&page, "<main>home</main>")?;
            Ok("one page".to_string())
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

        fn work_in(&self, _: &str, prompt: &str, dir: &Path) -> Result<String, String> {
            std::thread::sleep(Duration::from_millis(300));
            if let Some(page) = asked_page(prompt) {
                std::fs::create_dir_all(dir.join("mock/pages")).map_err(|e| e.to_string())?;
                std::fs::write(dir.join(page), "<main>late</main>").map_err(|e| e.to_string())?;
            }
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

    #[test]
    fn an_ai_that_only_answers_refuses_to_write() {
        assert!(Talker.work_in("m", "p", Path::new("/")).is_err());
    }

    /// A scope with one screen.
    const SCOPE: &str = "## Goal\nproof\n\n## Screens\n\n- **Home** — the ledger\n";

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
        for _ in 0..1000 {
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
    #[allow(clippy::too_many_lines)]
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
            scope: SCOPE.to_string(),
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
            oneshot: Some(Arc::new(Drawer::new(false))),
            model: "m".to_string(),
            mockups: Some(mockups),
            kit: Some(kit),
            data: None,
            plant: None,
            pins: Some(Arc::new(Pins)),
            sessions: 2,
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
        let drawn = desk.record().expect("read").expect("a record");
        assert_eq!(drawn.mocked.as_deref(), Some("Home: one page"));
        assert_eq!(drawn.step, Step::Review);
        assert_eq!(
            drawn.reference.as_ref().map(|r| r.path.as_str()),
            Some("pages/home.html"),
            "drawn first, as the reference"
        );
        let drafts = drawn.drafts.as_ref().expect("drafted beside the pages");
        assert_eq!(drafts.tables(), ["line"]);
        assert_eq!(drafts.ports.len(), 2);
        assert!(dir.join("main_agent/mock/index.html").is_file());

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
            Some("pages/home.html: 1 micro-frontend(s)")
        );
        let cut = desk.record().expect("read").expect("a record");
        assert!(!desk.cut_stale(&cut).await, "every page is cut");
        let gaps = desk.gaps(&cut).await.expect("drafts read");
        assert_eq!(
            gaps.unused,
            ["ledger.addLine"],
            "no micro-frontend adds a line"
        );
        assert!(gaps.undrafted.is_empty(), "{gaps:?}");
        desk.componentize_now()
            .await
            .expect("done already: nothing runs");
        assert_eq!(desk.job().map(|j| j.work), Some(Work::Componentize));
        let read = desk.read_requirements().await.expect("read");
        assert_eq!(read.requirements.len(), 1);
        assert_eq!(read.requirements[0].title, "See the ledger");
        assert_eq!(
            read.requirements[0].pages,
            vec!["pages/home.html".to_string()]
        );
        assert_eq!(
            read.requirements[0].components,
            vec!["Home".to_string()],
            "a micro-frontend the mock-up has not is dropped"
        );

        initialized_up_to_the_board(&desk, &dir).await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The second half of the flow above: accepted, the record is kept in
    /// the repository, the mock-up and the design system published, and
    /// the initialization stops at the board it has not.
    async fn initialized_up_to_the_board(desk: &CreationDesk, dir: &Path) {
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
        let kept = std::fs::read_to_string(dir.join("main_agent/mock/proof-of-concept.json"))
            .expect("the record is written");
        let kept: serde_json::Value = serde_json::from_str(&kept).expect("json");
        assert_eq!(kept["requirements"][0]["title"], "See the ledger");
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
            mockups: None,
            kit: None,
            data: None,
            plant: None,
            pins: None,
            sessions: 2,
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
    async fn preparing_proposes_a_first_scope_and_says_what_the_design_system_waits_for() {
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
            oneshot: Some(Arc::new(Drawer::new(false))),
            model: "m".to_string(),
            mockups: Some(mockups),
            kit: Some(kit),
            data: None,
            plant: None,
            pins: None,
            sessions: 2,
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
        assert_eq!(prepared.scope, "## Goal\nfirst");
        assert!(!prepared.scope_stale() && !prepared.scope_edited);
        assert_eq!(prepared.step, Step::Brief, "a preparation moves no step");

        // Written already and not stale: proposed again by nothing.
        desk.prepare().await.expect("started");
        let job = settled(&desk).await;
        assert!(
            job.steps
                .iter()
                .any(|s| s == "the scope is written already"),
            "{:?}",
            job.steps
        );

        // A decision taken: the scope is stale, and a preparation proposes again.
        desk.save(
            Edit {
                questions: None,
                brief: Some("a ledger for two".to_string()),
                ..Edit::default()
            },
            None,
        )
        .await
        .expect("saved");
        assert!(
            desk.record()
                .expect("read")
                .expect("a record")
                .scope_stale()
        );
        desk.prepare().await.expect("started");
        let job = settled(&desk).await;
        assert!(
            job.steps
                .iter()
                .any(|s| s.starts_with("a first scope is proposed")),
            "{:?}",
            job.steps
        );
        assert!(
            !desk
                .record()
                .expect("read")
                .expect("a record")
                .scope_stale()
        );
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

    /// Claude for the preparation: the scope, the tokens, a component.
    struct Prepared;

    impl Oneshot for Prepared {
        fn ask(&self, _: &str, prompt: &str) -> Result<String, String> {
            if prompt.contains("You scope the proof of concept") {
                return Ok("## Goal\nfirst".to_string());
            }
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
            sessions: 2,
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
            scope: SCOPE.to_string(),
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
                sessions: 2,
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
        std::fs::write(
            dir.join("main_agent/mock/pages.json"),
            r#"{"version":1,"pages":[{"path":"pages/home.html","title":"Home"}],"components":[]}"#,
        )
        .expect("seeded");
        std::fs::create_dir_all(dir.join("main_agent/mock/pages")).expect("dir");
        std::fs::write(
            dir.join("main_agent/mock/pages/home.html"),
            "<main>inline</main>",
        )
        .expect("seeded");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let book = Arc::new(Book::default());
        let record = Creation {
            kind: Some(Kind::Vertical),
            brief: "a ledger".to_string(),
            scope: SCOPE.to_string(),
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
            oneshot: Some(Arc::new(Drawer::new(true))),
            model: "m".to_string(),
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
            sessions: 2,
        });
        let kit = desk.kit().await.expect("a kit");
        assert!(kit.validated);
        assert_eq!((kit.waiting, kit.generated, kit.running), (0, 0, false));
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

    /// Sessions that draw slowly enough for a poller's own 2s tick to land
    /// on each page in turn: the reference "A" at once, "B" seconds later,
    /// and "C" never — its session returns having written nothing.
    struct StagedPages;

    impl Oneshot for StagedPages {
        fn ask(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }

        fn work_in(&self, _: &str, prompt: &str, dir: &Path) -> Result<String, String> {
            let page = asked_page(prompt).ok_or("no page asked")?;
            let write = || -> Result<(), String> {
                std::fs::create_dir_all(dir.join("mock/pages")).map_err(|e| e.to_string())?;
                std::fs::write(dir.join(&page), "<main></main>").map_err(|e| e.to_string())
            };
            if page.ends_with("/a.html") {
                write()?;
            } else if page.ends_with("/b.html") {
                std::thread::sleep(DRAW_POLL + Duration::from_secs(1));
                write()?;
            } else {
                return Ok("nothing".to_string());
            }
            // Left on disk long enough for the poller's next tick to see it
            // before the session returns and the poller is aborted.
            std::thread::sleep(DRAW_POLL + Duration::from_secs(1));
            Ok("drawn".to_string())
        }
    }

    #[tokio::test]
    async fn drawing_the_mock_up_reports_each_page_live_and_names_one_never_drawn() {
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
            scope: "## Screens\n- **A** — first\n- **B** — second\n- **C** — third\n".to_string(),
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
            mockups: Some(mockups),
            kit: Some(kit),
            data: None,
            plant: None,
            pins: None,
            sessions: 2,
        });
        desk.draw_mockups().await.expect("started");
        // The reference's own line shows up while its session is still
        // running, well before it returns.
        let mut seen_a = false;
        for _ in 0..80 {
            if desk
                .job()
                .is_some_and(|j| j.steps.iter().any(|s| s.contains("\"A\" page (1 of 1)")))
            {
                seen_a = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(seen_a, "the reference's step never showed up live");
        assert_eq!(desk.job().map(|j| j.state), Some(State::Running));
        let job = settled(&desk).await;
        assert_eq!((job.work, job.state), (Work::Mockups, State::Failed));
        assert!(
            job.steps.iter().any(|s| s.contains("\"B\" page (1 of 2)")),
            "{:?}",
            job.steps
        );
        assert!(
            !job.steps.iter().any(|s| s.contains("\"C\" page")),
            "never written: never reported drawn — {:?}",
            job.steps
        );
        assert!(
            job.steps.iter().any(|s| s.contains("2 at a time")),
            "{:?}",
            job.steps
        );
        assert_eq!(
            job.why,
            "a page was not drawn — C: the session wrote no `mock/pages/c.html`"
        );
        assert!(
            desk.record()
                .expect("read")
                .expect("a record")
                .mocked
                .is_none(),
            "a page missing: not drawn"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A desk over a product whose design system is validated, with a
    /// creation at the scope's review and two screens.
    fn reviewing(tag: &str, scope: &str, drawer: Arc<Drawer>) -> (CreationDesk, PathBuf) {
        let dir = product(tag);
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
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
            scope: scope.to_string(),
            step: Step::Scope,
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
            oneshot: Some(drawer),
            model: "m".to_string(),
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
            sessions: 2,
        });
        (desk, dir)
    }

    const TWO: &str = "## Screens\n\n- **Home** — the ledger\n- **Lines** — every line\n";

    async fn scope_settled(desk: &CreationDesk, scope: &str) {
        desk.save(
            Edit {
                scope: Some(scope.to_string()),
                ..Edit::default()
            },
            None,
        )
        .await
        .expect("saved");
    }

    fn listed(dir: &Path) -> Vec<String> {
        let text =
            std::fs::read_to_string(dir.join("main_agent/mock/pages.json")).expect("pages.json");
        let root: serde_json::Value = serde_json::from_str(&text).expect("json");
        root["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .filter_map(|p| p["path"].as_str().map(ToString::to_string))
            .collect()
    }

    #[tokio::test]
    async fn the_reference_is_drawn_during_the_scope_review_then_kept_and_followed() {
        let drawer = Arc::new(Drawer::new(false));
        let (desk, dir) = reviewing("reference-kept", TWO, Arc::clone(&drawer));
        desk.draw_reference().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(
            (job.work, job.state),
            (Work::Reference, State::Done),
            "{}",
            job.why
        );
        let record = desk.record().expect("read").expect("a record");
        let reference = record.reference.clone().expect("kept");
        assert_eq!(
            (reference.screen.as_str(), reference.path.as_str()),
            ("Home", "pages/home.html")
        );
        assert_eq!(listed(&dir), ["pages/home.html"]);
        assert!(drawer.prompts()[0].contains("**reference**"));
        assert!(desk.draw_reference().await.is_err(), "drawn once");

        desk.draw_mockups().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(
            (job.work, job.state),
            (Work::Mockups, State::Done),
            "{}",
            job.why
        );
        assert!(
            job.steps.iter().any(|s| s.contains("\"Home\" is kept")),
            "{:?}",
            job.steps
        );
        let prompts = drawer.prompts();
        assert_eq!(prompts.len(), 2, "the reference is not drawn twice");
        assert!(prompts[1].contains("Read `mock/pages/home.html` first"));
        assert!(prompts[1].contains("Write `mock/pages/lines.html`"));
        assert_eq!(listed(&dir), ["pages/home.html", "pages/lines.html"]);
        let record = desk.record().expect("read").expect("a record");
        assert_eq!(record.mocked.as_deref(), Some("Lines: one page"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_reference_the_settled_scope_changed_is_redrawn_and_one_it_dropped_goes() {
        let drawer = Arc::new(Drawer::new(false));
        let (desk, dir) = reviewing("reference-changed", TWO, Arc::clone(&drawer));
        desk.draw_reference().await.expect("started");
        assert_eq!(settled(&desk).await.state, State::Done);
        scope_settled(
            &desk,
            "## Screens\n\n- **Home** — the ledger and its total\n- **Lines** — every line\n",
        )
        .await;
        desk.draw_mockups().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(job.state, State::Done, "{}", job.why);
        assert!(
            job.steps
                .iter()
                .any(|s| s.contains("drawn again, on its own look")),
            "{:?}",
            job.steps
        );
        let prompts = drawer.prompts();
        assert_eq!(prompts.len(), 3);
        assert!(prompts[1].contains("keeping its look"), "{}", prompts[1]);
        assert!(prompts[1].contains("the ledger and its total"));
        assert_eq!(listed(&dir), ["pages/home.html", "pages/lines.html"]);
        let _ = std::fs::remove_dir_all(&dir);

        let drawer = Arc::new(Drawer::new(false));
        let (desk, dir) = reviewing("reference-dropped", TWO, Arc::clone(&drawer));
        desk.draw_reference().await.expect("started");
        assert_eq!(settled(&desk).await.state, State::Done);
        scope_settled(&desk, "## Screens\n\n- **Lines** — every line\n").await;
        desk.draw_mockups().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(job.state, State::Done, "{}", job.why);
        assert!(
            job.steps
                .iter()
                .any(|s| s.contains("lends its look, then goes")),
            "{:?}",
            job.steps
        );
        assert!(drawer.prompts()[1].contains("Read `mock/pages/home.html` first"));
        assert_eq!(listed(&dir), ["pages/lines.html"]);
        assert!(!dir.join("main_agent/mock/pages/home.html").is_file());
        let index = std::fs::read_to_string(dir.join("main_agent/mock/index.html")).expect("index");
        assert!(index.contains("pages/lines.html") && !index.contains("pages/home.html"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn only_the_pages_moved_since_they_were_cut_are_cut_again() {
        let drawer = Arc::new(Drawer::new(false));
        let (desk, dir) = reviewing("recut", TWO, Arc::clone(&drawer));
        desk.draw_mockups().await.expect("started");
        assert_eq!(settled(&desk).await.state, State::Done);
        desk.componentize_now().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(job.state, State::Done, "{}", job.why);
        // Nothing cut yet: the first page alone, then the others.
        assert!(
            job.steps
                .iter()
                .any(|s| s.contains("make pages/home.html again")),
            "{:?}",
            job.steps
        );
        assert!(
            job.steps
                .iter()
                .any(|s| s.contains("make pages/lines.html again")),
            "{:?}",
            job.steps
        );
        let record = desk.record().expect("read").expect("a record");
        assert_eq!(record.cut.len(), 2);
        assert!(!desk.cut_stale(&record).await);
        // The human retouches one page: only that one is cut again.
        std::fs::write(
            dir.join("main_agent/mock/pages/lines.html"),
            "<main>retouched</main>",
        )
        .expect("retouched");
        assert!(desk.cut_stale(&record).await);
        let before = drawer.prompts().len();
        desk.componentize_now().await.expect("started");
        let job = settled(&desk).await;
        assert_eq!(job.state, State::Done, "{}", job.why);
        let cut: Vec<String> = drawer.prompts()[before..]
            .iter()
            .filter_map(|p| cut_page(p))
            .collect();
        assert_eq!(cut, ["mock/pages/lines.html"]);
        let record = desk.record().expect("read").expect("a record");
        assert!(!desk.cut_stale(&record).await);
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
                prompt.contains("again of **micro-frontends**"),
                "the right prompt"
            );
            assert!(
                prompt.contains("- Old ("),
                "the micro-frontends made already are listed"
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
                "mock/pages/home.html",
                r#"<div data-component="Old"></div><div data-component="New"></div>"#,
            )?;
            std::thread::sleep(DRAW_POLL + Duration::from_secs(1));
            Ok(r#"{"components":[{"name":"New","system":"ledger"}]}"#.to_string())
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
            scope: SCOPE.to_string(),
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
            mockups: Some(mockups),
            kit: Some(kit),
            data: None,
            plant: None,
            pins: None,
            sessions: 2,
        });
        desk.accept().await.expect("accepted");
        let job = settled(&desk).await;
        assert!(
            job.steps
                .iter()
                .any(|s| s.contains("\"New\" micro-frontend")),
            "{:?}",
            job.steps
        );
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
