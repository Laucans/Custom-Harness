//! The micro-frontends' generation, on the design system's model: a spec
//! in, a fragment of the mock-up out, several at once.
//!
//! One generation is three steps. **Plan**: one light question reads the
//! kit's index (`KitIndex`, never its code) and answers which components
//! the micro-frontend uses and which it lacks. **Kit**: the missing ones are
//! added to the kit as custom components and generated there first — the
//! design system's easel does it, as a human's would. **Write**: one
//! question reads the skeletons of the components used, the units' contracts
//! and rules, the fixtures of the Capabilities it needs that exist, and
//! answers the fragment (and the fixtures still missing, one per
//! Capability); the fragment, the fixtures and the manifest's entry are written in the branch's workspace, saved with the
//! mock-up like a human's edit. The questions are blocking: each one runs
//! in `spawn_blocking`; a generation is a task, and the sessions' count
//! bounds how many ask at once.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Semaphore;

use crate::domain::design;
use crate::domain::drafts::Asked;
use crate::domain::mf_gen::{self, Spec};
use crate::kit_desk::{JobState, KitDesk, KitIndex};
use crate::ports::{Mockups, Oneshot};

/// How long a generation waits for the kit to make what it lacks.
const KIT_PATIENCE: Duration = Duration::from_mins(20);

/// How often it looks.
const KIT_POLL: Duration = Duration::from_secs(3);

/// Why nothing generates.
const NO_CLAUDE: &str = "no Claude at hand: the view found no `claude` to ask";

/// What the desk is made of.
pub struct Setup {
    /// The product's mock-up, on every branch.
    pub mockups: Arc<dyn Mockups>,
    /// The design system's easel: what makes a missing component.
    pub kit: Option<KitDesk>,
    /// The model asked; `None` when no Claude is at hand.
    pub oneshot: Option<Arc<dyn Oneshot>>,
    /// Its name.
    pub model: String,
    /// How many generations ask at once.
    pub sessions: usize,
}

/// Where a generation stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Waiting for a free session.
    Queued,
    /// Asking which components it is made of.
    Planning,
    /// Waiting for the kit to make what it lacks.
    Kit,
    /// Asking for the fragment.
    Writing,
    /// Written in the branch's workspace.
    Done,
    /// It failed; `why` says what happened.
    Failed,
}

impl Step {
    const fn busy(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Planning | Self::Kit | Self::Writing
        )
    }
}

/// One micro-frontend's generation, as the page follows it.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    /// The branch it is written on.
    pub branch: String,
    /// The micro-frontend.
    pub name: String,
    /// Where it stands.
    pub step: Step,
    /// Why it failed, or what to know of it once done; empty otherwise.
    pub why: String,
    /// The kit's components it is made of.
    pub uses: Vec<String>,
    /// The components the kit made for it.
    pub made: Vec<String>,
}

/// The micro-frontends' generations.
#[derive(Clone)]
pub struct MfDesk {
    setup: Arc<Setup>,
    jobs: Arc<Mutex<Vec<Job>>>,
    gate: Arc<Semaphore>,
    /// One maker of missing components at a time: two plans that lack the
    /// same component make it once.
    making: Arc<tokio::sync::Mutex<()>>,
    /// Held around every write of a branch's mock-up files.
    files: Arc<Mutex<()>>,
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// One lock per micro-frontend being rewritten: requests applied to it
    /// at once run one after the other, each on the last one's draft.
    rewriting: Arc<Mutex<BTreeMap<String, Arc<Mutex<()>>>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MfDesk {
    /// A desk with nothing generating.
    #[must_use]
    pub fn new(setup: Setup) -> Self {
        let sessions = setup.sessions.max(1);
        Self {
            setup: Arc::new(setup),
            jobs: Arc::default(),
            gate: Arc::new(Semaphore::new(sessions)),
            making: Arc::default(),
            files: Arc::default(),
            tasks: Arc::default(),
            rewriting: Arc::default(),
        }
    }

    /// The generations asked on a branch, the last one of each name.
    #[must_use]
    pub fn jobs(&self, branch: &str) -> Vec<Job> {
        lock(&self.jobs)
            .iter()
            .filter(|j| j.branch == branch)
            .cloned()
            .collect()
    }

    /// Queues the generation of each spec on `branch`, with the brief of its
    /// units; a micro-frontend already generating is left to it. Answers how
    /// many were queued.
    ///
    /// # Errors
    ///
    /// No Claude at hand, a name that is not one, or nothing to queue.
    // The jobs' lock is held across its loop on purpose, as the tasks' is
    // across theirs: each iteration must see what the previous one queued.
    #[allow(clippy::significant_drop_tightening)]
    pub fn start(&self, branch: &str, items: Vec<(Spec, String)>) -> Result<usize, String> {
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        if let Some((bad, _)) = items
            .iter()
            .find(|(s, _)| !harness_architecture::kit::is_name(&s.name))
        {
            return Err(format!(
                "`{}` is not a micro-frontend's name: a capital, then letters and digits",
                bad.name
            ));
        }
        let mut queued = Vec::new();
        {
            let mut jobs = lock(&self.jobs);
            for (spec, units) in items {
                if jobs
                    .iter()
                    .any(|j| j.branch == branch && j.name == spec.name && j.step.busy())
                {
                    continue;
                }
                jobs.retain(|j| !(j.branch == branch && j.name == spec.name));
                jobs.push(Job {
                    branch: branch.to_string(),
                    name: spec.name.clone(),
                    step: Step::Queued,
                    why: String::new(),
                    uses: Vec::new(),
                    made: Vec::new(),
                });
                queued.push((spec, units));
            }
        }
        if queued.is_empty() {
            return Err("nothing to generate: each one asked is generating already".to_string());
        }
        let count = queued.len();
        let mut tasks = lock(&self.tasks);
        tasks.retain(|t| !t.is_finished());
        for (spec, units) in queued {
            let desk = self.clone();
            let oneshot = Arc::clone(&oneshot);
            let branch = branch.to_string();
            tasks.push(tokio::spawn(async move {
                let Ok(_permit) = desk.gate.clone().acquire_owned().await else {
                    return;
                };
                let name = spec.name.clone();
                let outcome = desk.generate(&branch, spec, &units, oneshot).await;
                desk.update(&branch, &name, |job| match outcome {
                    Ok(note) => {
                        job.step = Step::Done;
                        job.why = note;
                    }
                    Err(why) => {
                        job.step = Step::Failed;
                        job.why = why;
                    }
                });
            }));
        }
        Ok(count)
    }

    /// Applies requests to a micro-frontend: its draft — or its fragment
    /// the first time — rewritten by one question, kept as its draft.
    /// Answers what was skipped, `(id, why)`.
    ///
    /// # Errors
    ///
    /// No Claude at hand, no request, no fragment yet, or the answer
    /// carries none.
    pub async fn apply(
        &self,
        branch: &str,
        spec: Spec,
        units: String,
        asked: Vec<Asked>,
    ) -> Result<Vec<(String, String)>, String> {
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        if asked.is_empty() {
            return Err("no request to apply".to_string());
        }
        let index = self.index().await?;
        let model = self.setup.model.clone();
        let mockups = Arc::clone(&self.setup.mockups);
        let files = Arc::clone(&self.files);
        let rewrite = {
            let mut locks = lock(&self.rewriting);
            Arc::clone(locks.entry(format!("{branch}/{}", spec.name)).or_default())
        };
        let branch = branch.to_string();
        tokio::task::spawn_blocking(move || {
            // Held from the read to the write: a second request on it waits, then starts from this draft.
            let _rewriting = lock(&rewrite);
            let text = |path: &str| -> Result<Option<String>, String> {
                Ok(mockups
                    .file(&branch, path)?
                    .map(|b| String::from_utf8_lossy(&b).into_owned()))
            };
            let fragment = text(&mf_gen::draft_path(&spec.name))?
                .or(text(&mf_gen::fragment_path(&spec.name))?)
                .ok_or_else(|| format!("{} has no fragment yet: generate it first", spec.name))?;
            let prompt = mf_gen::rewrite_prompt(
                &spec,
                &units,
                &index.lines(),
                &index.tokens.join(", "),
                &fragment,
                &asked,
            );
            let rewritten = mf_gen::generated_of(&oneshot.ask(&model, &prompt)?, &[])?.fragment;
            let skipped = design::skipped_in(&rewritten);
            let _held = lock(&files);
            mockups.write(
                &branch,
                &mf_gen::draft_path(&spec.name),
                rewritten.as_bytes(),
            )?;
            Ok(skipped)
        })
        .await
        .map_err(|e| format!("the rewrite was dropped: {e}"))?
    }

    /// The micro-frontends of a branch that have a draft, among `names`.
    ///
    /// # Errors
    ///
    /// The repository cannot be read.
    pub async fn drafts(&self, branch: &str, names: Vec<String>) -> Result<Vec<String>, String> {
        let mockups = Arc::clone(&self.setup.mockups);
        let branch = branch.to_string();
        tokio::task::spawn_blocking(move || {
            let mut with = Vec::new();
            for name in names {
                if mockups.file(&branch, &mf_gen::draft_path(&name))?.is_some() {
                    with.push(name);
                }
            }
            Ok(with)
        })
        .await
        .map_err(|e| format!("the read was dropped: {e}"))?
    }

    /// Saves a micro-frontend's draft: in its place — every page that
    /// injects it shows the new one — or as a fork, a new micro-frontend
    /// listed beside it, the origin untouched. A fork made from a page
    /// (`page`, relative to `mock/`) is injected there in place of the
    /// origin. Answers the name saved as.
    ///
    /// # Errors
    ///
    /// No draft, a fork's name taken or not a name, or the workspace could
    /// not be written.
    pub async fn save(
        &self,
        branch: &str,
        name: String,
        fork: Option<String>,
        page: Option<String>,
    ) -> Result<String, String> {
        if let Some(wanted) = fork
            .as_deref()
            .filter(|w| !w.is_empty() && !harness_architecture::kit::is_name(w))
        {
            return Err(format!(
                "`{wanted}` is not a micro-frontend's name: a capital, then letters and digits"
            ));
        }
        let mockups = Arc::clone(&self.setup.mockups);
        let files = Arc::clone(&self.files);
        let branch = branch.to_string();
        tokio::task::spawn_blocking(move || {
            let _held = lock(&files);
            let draft = mockups
                .file(&branch, &mf_gen::draft_path(&name))?
                .ok_or_else(|| format!("{name} has no draft to save"))?;
            let saved_as = match fork {
                None => {
                    mockups.write(&branch, &mf_gen::fragment_path(&name), &draft)?;
                    name.clone()
                }
                Some(wanted) => {
                    let pages = mockups
                        .file(&branch, "pages.json")?
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .ok_or("the mock-up has no pages.json to list a fork in")?;
                    let to = if wanted.is_empty() {
                        mf_gen::fork_name(&pages, &name)
                    } else {
                        wanted
                    };
                    let mut listed = mf_gen::forked(&pages, &name, &to)?;
                    mockups.write(&branch, &mf_gen::fragment_path(&to), &draft)?;
                    if let Some(page) = page.filter(|p| !p.is_empty()) {
                        listed = mf_gen::page_forked(&listed, &page, &name, &to)?;
                        let html = mockups
                            .file(&branch, &page)?
                            .map(|b| String::from_utf8_lossy(&b).into_owned())
                            .ok_or_else(|| format!("the mock-up has no {page}"))?;
                        mockups.write(
                            &branch,
                            &page,
                            mf_gen::retargeted(&html, &name, &to).as_bytes(),
                        )?;
                    }
                    mockups.write(&branch, "pages.json", listed.as_bytes())?;
                    to
                }
            };
            mockups.remove(&branch, &mf_gen::draft_path(&name))?;
            Ok(saved_as)
        })
        .await
        .map_err(|e| format!("the save was dropped: {e}"))?
    }

    /// Drops a micro-frontend's draft: its fragment as it was.
    ///
    /// # Errors
    ///
    /// The workspace could not be written.
    pub async fn drop_draft(&self, branch: &str, name: String) -> Result<(), String> {
        let mockups = Arc::clone(&self.setup.mockups);
        let files = Arc::clone(&self.files);
        let branch = branch.to_string();
        tokio::task::spawn_blocking(move || {
            let _held = lock(&files);
            mockups.remove(&branch, &mf_gen::draft_path(&name))
        })
        .await
        .map_err(|e| format!("the drop was dropped: {e}"))?
    }

    /// Stops every generation: the view is going away.
    pub fn shutdown(&self) {
        for task in lock(&self.tasks).drain(..) {
            task.abort();
        }
    }

    fn update(&self, branch: &str, name: &str, change: impl FnOnce(&mut Job)) {
        if let Some(job) = lock(&self.jobs)
            .iter_mut()
            .find(|j| j.branch == branch && j.name == name)
        {
            change(job);
        }
    }

    /// One generation, its three steps. Answers what to know of it once
    /// written: the parts it left without their unit, if any.
    async fn generate(
        &self,
        branch: &str,
        spec: Spec,
        units: &str,
        oneshot: Arc<dyn Oneshot>,
    ) -> Result<String, String> {
        let model = self.setup.model.clone();
        self.update(branch, &spec.name, |j| j.step = Step::Planning);
        let mut index = self.index().await?;
        let prompt = mf_gen::plan_prompt(&spec, units, &index.lines());
        let plan = mf_gen::plan_of(&ask(&oneshot, &model, prompt).await?)?;
        let mut uses: Vec<String> = plan
            .uses
            .iter()
            .filter(|u| index.components.iter().any(|(name, _, _)| name == *u))
            .cloned()
            .collect();
        if !plan.missing.is_empty() {
            self.update(branch, &spec.name, |j| j.step = Step::Kit);
            let made = self.make_missing(&plan.missing, &index).await?;
            self.update(branch, &spec.name, |j| j.made.clone_from(&made));
            uses.extend(made);
            index = self.index().await?;
        }
        self.update(branch, &spec.name, |j| {
            j.uses.clone_from(&uses);
            j.step = Step::Writing;
        });
        // One fixture per Capability it needs: those there are read, the
        // others asked for.
        let mut fixtures = Vec::new();
        for need in &spec.needs {
            let text = self.read(branch, &mf_gen::fixture_path(need)).await?;
            fixtures.push((need.clone(), text));
        }
        let missing: Vec<String> = fixtures
            .iter()
            .filter(|(_, text)| text.is_none())
            .map(|(need, _)| need.clone())
            .collect();
        let prompt = mf_gen::generate_prompt(
            &spec,
            units,
            &index.skeletons(&uses),
            &index.tokens.join(", "),
            &fixtures,
        );
        let mut generated =
            mf_gen::generated_of(&ask(&oneshot, &model, prompt.clone()).await?, &missing)?;
        let mut untagged = mf_gen::untagged(&generated.fragment, &spec);
        // The gates' check on the fragment, the answer asked again until it
        // passes.
        let mut retags = 0;
        while !untagged.is_empty() && retags < mf_gen::RETAGS {
            retags += 1;
            let again = mf_gen::retag_prompt(&prompt, &generated.fragment, &untagged);
            generated = mf_gen::generated_of(&ask(&oneshot, &model, again).await?, &missing)?;
            untagged = mf_gen::untagged(&generated.fragment, &spec);
        }
        let mockups = Arc::clone(&self.setup.mockups);
        let files = Arc::clone(&self.files);
        let branch_owned = branch.to_string();
        let paths: Vec<String> = spec.needs.iter().map(|n| mf_gen::fixture_path(n)).collect();
        tokio::task::spawn_blocking(move || {
            let _held = lock(&files);
            mockups.write(
                &branch_owned,
                &mf_gen::fragment_path(&spec.name),
                generated.fragment.as_bytes(),
            )?;
            for (capability, text) in &generated.fixtures {
                mockups.write(
                    &branch_owned,
                    &mf_gen::fixture_path(capability),
                    text.as_bytes(),
                )?;
            }
            let pages = mockups
                .file(&branch_owned, "pages.json")?
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            let listed = mf_gen::listed(pages.as_deref(), &spec, &paths)?;
            mockups.write(&branch_owned, "pages.json", listed.as_bytes())
        })
        .await
        .map_err(|e| format!("the generation's write was dropped: {e}"))??;
        Ok(if untagged.is_empty() {
            String::new()
        } else {
            format!(
                "no part names {}: ask for it with a request",
                untagged.join(", ")
            )
        })
    }

    async fn index(&self) -> Result<KitIndex, String> {
        match &self.setup.kit {
            Some(kit) => kit.index().await,
            None => Ok(KitIndex::default()),
        }
    }

    async fn read(&self, branch: &str, path: &str) -> Result<Option<String>, String> {
        let mockups = Arc::clone(&self.setup.mockups);
        let (branch, path) = (branch.to_string(), path.to_string());
        tokio::task::spawn_blocking(move || {
            Ok(mockups
                .file(&branch, &path)?
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
        })
        .await
        .map_err(|e| format!("the read was dropped: {e}"))?
    }

    /// The components a plan lacks, made by the kit — one maker at a time,
    /// so what another plan just made is reused. Answers their names.
    async fn make_missing(
        &self,
        missing: &[mf_gen::Missing],
        index: &KitIndex,
    ) -> Result<Vec<String>, String> {
        let kit = self.setup.kit.clone().ok_or("the micro-frontend needs components the kit lacks, and nothing reads the design system")?;
        if !index.validated {
            return Err(format!(
                "it needs {} the kit lacks: validate the design system first, the kit is made from it",
                missing
                    .iter()
                    .map(|m| m.title.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let _making = self.making.lock().await;
        let mut names = Vec::new();
        for m in missing {
            let name = match kit.titled(m.title.clone()).await? {
                Some(name) => name,
                None => {
                    kit.custom(String::new(), m.title.clone(), m.what.clone())
                        .await?
                        .name
                }
            };
            names.push(name);
        }
        let waiting: Vec<String> = {
            let ready = kit.index().await?;
            names
                .iter()
                .filter(|n| !ready.components.iter().any(|(name, _, _)| name == *n))
                .cloned()
                .collect()
        };
        if !waiting.is_empty() {
            match kit.start_generation(waiting.clone()).await {
                Ok(_) => {}
                // Generating already: the wait below follows it.
                Err(why) if why.starts_with("nothing to generate") => {}
                Err(why) => return Err(why),
            }
            let deadline = tokio::time::Instant::now() + KIT_PATIENCE;
            loop {
                let states: Vec<_> = waiting.iter().map(|n| (n, kit.job_of(n))).collect();
                if let Some((name, Some((_, why)))) = states
                    .iter()
                    .find(|(_, s)| matches!(s, Some((JobState::Failed, _))))
                {
                    return Err(format!("the kit could not make {name}: {why}"));
                }
                if states
                    .iter()
                    .all(|(_, s)| matches!(s, Some((JobState::Done, _))))
                {
                    break;
                }
                if tokio::time::Instant::now() > deadline {
                    return Err(format!(
                        "the kit took too long to make {}",
                        waiting.join(", ")
                    ));
                }
                tokio::time::sleep(KIT_POLL).await;
            }
        }
        Ok(names)
    }
}

/// One blocking question, off the runtime's thread.
async fn ask(oneshot: &Arc<dyn Oneshot>, model: &str, prompt: String) -> Result<String, String> {
    let oneshot = Arc::clone(oneshot);
    let model = model.to_string();
    tokio::task::spawn_blocking(move || oneshot.ask(&model, &prompt))
        .await
        .map_err(|e| format!("the question was dropped: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mockups::DirMockups;
    use crate::domain::drafts::Asked;
    use crate::kit_desk;
    use crate::kit_desk::tests::Shelf;
    use crate::ports::NoteStore;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// A model that plans, makes a kit component and writes a micro-frontend,
    /// keeping every question it was asked.
    #[derive(Default)]
    struct Model {
        asked: Mutex<Vec<String>>,
        /// It never names its actuator, however often it is asked.
        stubborn: bool,
    }

    impl Oneshot for Model {
        fn ask(&self, _model: &str, prompt: &str) -> Result<String, String> {
            lock(&self.asked).push(prompt.to_string());
            if prompt.starts_with("You plan one micro-frontend") && self.stubborn {
                return Ok(r#"{"uses": [], "missing": []}"#.to_string());
            }
            if prompt.starts_with("You plan one micro-frontend") {
                return Ok(r#"{"uses": ["Button"], "missing": [{"title": "Fiche", "what": "a character sheet"}]}"#.to_string());
            }
            if prompt.starts_with("You edit one micro-frontend") {
                let n = lock(&self.asked).len();
                return Ok(format!(
                    "```html\n<ul data-reads=\"campagne.listerPjs\" data-v=\"{n}\"></ul>\n<!-- skipped r2: needs data it does not read -->\n```"
                ));
            }
            // The first answer forgets its actuator's name; the gates send
            // it back.
            if prompt.starts_with("You write one micro-frontend")
                && (self.stubborn || !prompt.contains("refused by the gates"))
            {
                return Ok("```html\n<section><button class=\"primary\">Ajouter</button><ul data-reads=\"campagne.listerPjs\"></ul></section>\n```\n```json\n{\"campagne.listerPjs\": []}\n```".to_string());
            }
            if prompt.starts_with("You write one micro-frontend") {
                return Ok("```html\n<section><button class=\"primary\" data-writes=\"campagne.ajouterPj\">Ajouter</button><ul data-reads=\"campagne.listerPjs\"></ul></section>\n```\n```json\n{\"campagne.listerPjs\": [{\"id\": 1, \"nom\": \"Aldric\"}]}\n```".to_string());
            }
            Ok("```html\n<div class=\"kit-fiche\" style=\"color:var(--color-text, #111111)\">Fiche</div>\n```".to_string())
        }
    }

    fn product(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("harness-mf-desk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |path: &str, text: &str| {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().expect("a folder")).expect("folder");
            std::fs::write(full, text).expect("write");
        };
        write(
            "main/mock/pages.json",
            r#"{"version":1,"pages":[],"components":[]}"#,
        );
        write(
            "design-system/mock/design/design.json",
            r#"{"version":1,"state":"validated"}"#,
        );
        write(
            "design-system/mock/design/tokens.css",
            ":root{--color-primary:#2b6cb0;--space-2:8px;}\n",
        );
        write(
            "design-system/mock/kit/kit.json",
            r#"{"version":1,"checked":[],"components":[{"name":"Button","base":"button","title":"Button","state":"generated"}]}"#,
        );
        write(
            "design-system/mock/kit/Button.html",
            harness_architecture::kit::base("button")
                .expect("button")
                .fragment,
        );
        dir
    }

    // One story, end to end: the plan asks for a component the kit lacks,
    // the kit makes it, the micro-frontend is written with both.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn a_micro_frontend_is_planned_its_missing_component_made_then_written() {
        let dir = product("planned");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let model = Arc::new(Model::default());
        let pins: Arc<dyn NoteStore> = Arc::new(Shelf::default());
        let kit = KitDesk::new(kit_desk::Setup {
            mockups: Arc::clone(&mockups),
            oneshot: Some(model.clone() as Arc<dyn Oneshot>),
            model: "m".into(),
            sessions: 1,
            pins,
            integration_branch: "main".into(),
        });
        let desk = MfDesk::new(Setup {
            mockups: Arc::clone(&mockups),
            kit: Some(kit),
            oneshot: Some(model.clone() as Arc<dyn Oneshot>),
            model: "m".into(),
            sessions: 2,
        });
        let spec = Spec {
            name: "Pjs".into(),
            system: "campagne".into(),
            description: "Liste et ajoute les PJ".into(),
            needs: vec!["campagne.listerPjs".into()],
            actions: vec!["campagne.ajouterPj".into()],
            props: BTreeMap::new(),
        };
        assert!(
            desk.start(
                "main",
                vec![(
                    Spec {
                        name: "pjs".into(),
                        ..spec.clone()
                    },
                    String::new()
                )]
            )
            .is_err(),
            "a name is a capital first"
        );
        assert_eq!(
            desk.start("main", vec![(spec.clone(), "- units\n".into())])
                .expect("queued"),
            1
        );
        assert!(
            desk.start("main", vec![(spec, String::new())]).is_err(),
            "generating already"
        );
        let mut job = None;
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let now = desk.jobs("main").into_iter().next();
            if now.as_ref().is_some_and(|j| !j.step.busy()) {
                job = now;
                break;
            }
        }
        let job = job.expect("the generation ended");
        let read = |path: &str| std::fs::read_to_string(dir.join(path)).unwrap_or_default();
        let asked = lock(&model.asked).clone();
        let fragment = read("main/mock/components/Pjs.html");
        let fixture = read("main/mock/fixtures/campagne.listerPjs.json");
        let pages = read("main/mock/pages.json");
        let kit_json = read("design-system/mock/kit/kit.json");
        let made = read("design-system/mock/kit/Fiche.html");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            fragment.contains("data-writes=\"campagne.ajouterPj\""),
            "{fragment}"
        );
        assert!(fixture.contains("Aldric"), "{fixture}");
        assert!(
            pages.contains("\"fragment\": \"components/Pjs.html\"")
                && pages.contains("\"fixtures/campagne.listerPjs.json\"")
                && !pages.contains("\"fixture\":"),
            "{pages}"
        );
        assert!(
            kit_json.contains("\"Fiche\"") && made.contains("kit-fiche"),
            "{kit_json}"
        );
        assert_eq!(job.step, Step::Done, "{}", job.why);
        assert_eq!(job.made, ["Fiche"]);
        assert_eq!(job.uses, ["Button", "Fiche"]);
        assert!(job.why.is_empty(), "every part named: {}", job.why);
        // The plan read the kit's index, never its code; the generation the skeletons.
        let plan = asked
            .iter()
            .find(|p| p.starts_with("You plan"))
            .expect("planned");
        assert!(
            plan.contains("- Button (Button)") && !plan.contains("<style"),
            "{plan}"
        );
        let write = asked
            .iter()
            .find(|p| p.starts_with("You write one micro-frontend"))
            .expect("written");
        assert!(
            write.contains("Button: <div class=\"kit-button\">")
                && write.contains("Fiche: <div class=\"kit-fiche\">"),
            "{write}"
        );
        assert!(write.contains("--color-primary") && !write.contains(".kit-button {"));
        let again: Vec<&String> = asked
            .iter()
            .filter(|p| p.contains("refused by the gates"))
            .collect();
        assert_eq!(again.len(), 1, "asked again once");
        assert!(
            again[0].contains("names campagne.ajouterPj.")
                && again[0].contains("<button class=\"primary\">Ajouter</button>"),
            "{}",
            again[0]
        );
    }

    // A model that never names its actuator is asked again `RETAGS` times,
    // then the fragment is written and the gap said.
    #[tokio::test]
    async fn a_fragment_the_gates_keep_refusing_is_asked_again_then_said() {
        let dir = product("stubborn");
        let mockups: Arc<dyn Mockups> = Arc::new(DirMockups::new(&dir));
        let model = Arc::new(Model {
            stubborn: true,
            ..Model::default()
        });
        let desk = MfDesk::new(Setup {
            mockups: Arc::clone(&mockups),
            kit: None,
            oneshot: Some(model.clone() as Arc<dyn Oneshot>),
            model: "m".into(),
            sessions: 1,
        });
        let spec = Spec {
            name: "Pjs".into(),
            system: "campagne".into(),
            description: "Liste et ajoute les PJ".into(),
            needs: vec!["campagne.listerPjs".into()],
            actions: vec!["campagne.ajouterPj".into()],
            props: BTreeMap::new(),
        };
        desk.start("main", vec![(spec, String::new())])
            .expect("queued");
        let mut job = None;
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let now = desk.jobs("main").into_iter().next();
            if now.as_ref().is_some_and(|j| !j.step.busy()) {
                job = now;
                break;
            }
        }
        let job = job.expect("the generation ended");
        let written = dir.join("main/mock/components/Pjs.html").is_file();
        let _ = std::fs::remove_dir_all(&dir);
        let again = lock(&model.asked)
            .iter()
            .filter(|p| p.contains("refused by the gates"))
            .count();
        assert_eq!(again, mf_gen::RETAGS as usize);
        assert_eq!(job.step, Step::Done, "{}", job.why);
        assert_eq!(
            job.why,
            "no part names campagne.ajouterPj: ask for it with a request"
        );
        assert!(written);
    }

    /// One request asking a micro-frontend's list to be bolder.
    fn bolder(id: &str) -> Vec<Asked> {
        vec![Asked {
            id: id.into(),
            text: "bolder".into(),
            target: "ul:nth-child(1)".into(),
            ..Asked::default()
        }]
    }

    // One scenario, three outcomes on one fixture: splitting it would copy
    // the fixture three times for no extra finding.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn requests_rewrite_a_draft_saved_in_place_or_as_a_fork_or_dropped() {
        let dir = std::env::temp_dir().join(format!("harness-mf-draft-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("main/mock/components")).expect("dir");
        std::fs::write(
            dir.join("main/mock/components/Pjs.html"),
            "<ul data-reads=\"campagne.listerPjs\"></ul>",
        )
        .expect("fragment");
        std::fs::write(
            dir.join("main/mock/pages.json"),
            r#"{"version":1,"pages":[{"path":"pages/a.html","components":["Pjs"]}],"components":[{"name":"Pjs","fragment":"components/Pjs.html","description":"Les PJ"}]}"#,
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.join("main/mock/pages")).expect("pages dir");
        std::fs::write(
            dir.join("main/mock/pages/a.html"),
            "<main><div data-component=\"Pjs\"></div></main>",
        )
        .expect("page");
        let desk = MfDesk::new(Setup {
            mockups: Arc::new(DirMockups::new(&dir)),
            kit: None,
            oneshot: Some(Arc::new(Model::default()) as Arc<dyn Oneshot>),
            model: "m".into(),
            sessions: 1,
        });
        let spec = Spec {
            name: "Pjs".into(),
            needs: vec!["campagne.listerPjs".into()],
            ..Spec::default()
        };
        let asked = bolder;
        let read = |path: &str| {
            std::fs::read_to_string(dir.join("main/mock").join(path)).unwrap_or_default()
        };
        assert!(
            desk.apply("main", spec.clone(), String::new(), vec![])
                .await
                .is_err(),
            "no request"
        );
        let skipped = desk
            .apply("main", spec.clone(), String::new(), asked("r1"))
            .await
            .expect("applied");
        assert_eq!(
            skipped,
            [("r2".to_string(), "needs data it does not read".to_string())]
        );
        assert!(
            read("components/Pjs.draft.html").contains("data-v"),
            "the draft is written"
        );
        assert_eq!(
            read("components/Pjs.html"),
            "<ul data-reads=\"campagne.listerPjs\"></ul>",
            "the fragment untouched"
        );
        assert_eq!(
            desk.drafts("main", vec!["Pjs".into(), "Other".into()])
                .await
                .expect("drafts"),
            ["Pjs"]
        );
        // As a fork: a new micro-frontend beside it, the origin untouched.
        assert_eq!(
            desk.save("main", "Pjs".into(), Some(String::new()), None)
                .await
                .expect("forked"),
            "Pjs2"
        );
        assert!(
            read("components/Pjs2.html").contains("data-v")
                && read("pages.json").contains("\"Pjs2\"")
        );
        assert!(
            read("components/Pjs.draft.html").is_empty(),
            "the draft goes once saved"
        );
        assert!(
            desk.save("main", "Pjs".into(), None, None).await.is_err(),
            "no draft left"
        );
        // In place.
        desk.apply("main", spec.clone(), String::new(), asked("r3"))
            .await
            .expect("applied again");
        assert_eq!(
            desk.save("main", "Pjs".into(), None, None)
                .await
                .expect("replaced"),
            "Pjs"
        );
        assert!(read("components/Pjs.html").contains("data-v"));
        // Dropped.
        desk.apply("main", spec.clone(), String::new(), asked("r4"))
            .await
            .expect("applied once more");
        desk.drop_draft("main", "Pjs".into())
            .await
            .expect("dropped");
        assert!(
            read("components/Pjs.draft.html").is_empty(),
            "expected nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_draft_forked_for_one_page_is_injected_there() {
        let dir = std::env::temp_dir().join(format!("harness-mf-page-fork-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("main/mock/components")).expect("dir");
        std::fs::write(
            dir.join("main/mock/components/Pjs.html"),
            "<ul data-reads=\"campagne.listerPjs\"></ul>",
        )
        .expect("fragment");
        std::fs::write(
            dir.join("main/mock/pages.json"),
            r#"{"version":1,"pages":[{"path":"pages/a.html","components":["Pjs"]}],"components":[{"name":"Pjs","fragment":"components/Pjs.html","description":"Les PJ"}]}"#,
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.join("main/mock/pages")).expect("pages dir");
        std::fs::write(
            dir.join("main/mock/pages/a.html"),
            "<main><div data-component=\"Pjs\"></div></main>",
        )
        .expect("page");
        let desk = MfDesk::new(Setup {
            mockups: Arc::new(DirMockups::new(&dir)),
            kit: None,
            oneshot: Some(Arc::new(Model::default()) as Arc<dyn Oneshot>),
            model: "m".into(),
            sessions: 1,
        });
        let spec = Spec {
            name: "Pjs".into(),
            needs: vec!["campagne.listerPjs".into()],
            ..Spec::default()
        };
        let asked = bolder;
        let read = |path: &str| {
            std::fs::read_to_string(dir.join("main/mock").join(path)).unwrap_or_default()
        };
        // As a fork for one page: that page injects the fork.
        desk.apply("main", spec, String::new(), asked("r5"))
            .await
            .expect("applied for a page");
        assert_eq!(
            desk.save(
                "main",
                "Pjs".into(),
                Some(String::new()),
                Some("pages/a.html".into())
            )
            .await
            .expect("forked for the page"),
            "Pjs2"
        );
        assert_eq!(
            read("pages/a.html"),
            "<main><div data-component=\"Pjs2\"></div></main>"
        );
        assert!(read("pages.json").contains("\"components\": [\n        \"Pjs2\""));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
