//! The design system's easel at work: the `design-system` branch of the
//! product read and edited through the `Mockups` port — its workspace made
//! on the first visit, the branch itself born on it — the tokens written
//! and derived, the moodboard kept, the generation asked of Claude in one
//! shot, the kit's components generated several at once on blocking
//! threads, their drafts rewritten from the canvas's requests, saved as a
//! fork or a replacement, and who uses each one read across the branches.
//!
//! The rules are `domain::design`'s and `harness_architecture::kit`'s;
//! the files, Claude and the pins are ports. This is the part with a
//! clock, threads and a lock: every read-modify-write of `design.json`,
//! `kit.json` and the tokens goes through one mutex, taken on a blocking
//! thread and never held across an `.await`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use harness_architecture::kit::{
    self, Component, ComponentState, Design, DesignState, Kit, Source,
};
use serde::Serialize;

use crate::domain::design::{self, DesignView, Entry, Usage};
use crate::domain::drafts::Asked;
use crate::ports::{Mockups, NoteStore, Oneshot};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The clock, as the traces write it.
fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// What the easel is made of.
pub struct Setup {
    /// The product's mock-up on every branch, the design system's among
    /// them.
    pub mockups: Arc<dyn Mockups>,
    /// Asks Claude once — `None` when no Claude is at hand: nothing
    /// generates, everything else works.
    pub oneshot: Option<Arc<dyn Oneshot>>,
    /// The model the generations run on.
    pub model: String,
    /// How many components are generated at once.
    pub sessions: usize,
    /// Where the canvas's pins are kept: the recoded ones are dropped here.
    pub pins: Arc<dyn NoteStore>,
    /// The branch the design system's is born from when it does not exist.
    pub integration_branch: String,
}

/// Where a generation job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Waiting for a free session.
    Queued,
    /// A session is on it.
    Running,
    /// Its fragment is written.
    Done,
    /// It failed; `why` says what happened.
    Failed,
}

/// One component's generation, as the page follows it.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    /// The component.
    pub name: String,
    /// Where it stands.
    pub state: JobState,
    /// Why it failed, or empty.
    pub why: String,
    /// When it started, or empty.
    pub started: String,
    /// When it ended, or empty.
    pub ended: String,
}

/// How a draft is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Save {
    /// The draft takes the component's place.
    Replace,
    /// The draft becomes a new component, the origin untouched.
    Fork {
        /// Its name, or empty for the next free `<origin>2`, `<origin>3`…
        name: String,
    },
}

/// An image the easel takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asset {
    /// One of the moodboard's.
    Moodboard,
    /// The logo.
    Logo,
    /// A custom component's reference drawing.
    Reference {
        /// The component.
        component: String,
    },
}

/// The easel; cheap to clone, shared by the handlers.
#[derive(Clone)]
pub struct KitDesk {
    setup: Arc<Setup>,
    /// Held around every read-modify-write of the design system's files.
    files: Arc<Mutex<()>>,
    jobs: Arc<Mutex<Vec<Job>>>,
    running: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    workers: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// One lock per component being rewritten: requests applied to the same
    /// component at once run one after the other, each on the last one's
    /// draft, so none is lost.
    rewriting: Arc<Mutex<BTreeMap<String, Arc<Mutex<()>>>>>,
}

/// The kit as a micro-frontend's generation reads it, without its code.
#[derive(Debug, Clone, Default)]
pub struct KitIndex {
    /// The design system is validated: missing components may be generated.
    pub validated: bool,
    /// Every generated component: its name, its index line, its skeleton.
    pub components: Vec<(String, String, String)>,
    /// The design system's variables, by name.
    pub tokens: Vec<String>,
}

impl KitIndex {
    /// The index, one line a component.
    #[must_use]
    pub fn lines(&self) -> String {
        self.components
            .iter()
            .fold(String::new(), |mut out, (_, line, _)| {
                let _ = writeln!(out, "{line}");
                out
            })
    }

    /// The skeletons of the components named, those the kit has.
    #[must_use]
    pub fn skeletons(&self, names: &[String]) -> String {
        self.components
            .iter()
            .filter(|(name, _, _)| names.iter().any(|n| n == name))
            .fold(String::new(), |mut out, (name, _, shape)| {
                let _ = writeln!(out, "{name}: {shape}");
                out
            })
    }
}

/// A custom component added: the name it was given, and the catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct Created {
    /// The component's name, made from its title unless one was given.
    pub name: String,
    /// The catalogue with it.
    pub catalog: Vec<Entry>,
}

/// A publication: the commit on the design system's branch, and what
/// became of carrying it onto the integration branch.
#[derive(Debug, Clone, Serialize)]
pub struct Published {
    /// The commit on `design-system`.
    pub commit: String,
    /// The branch it is carried onto.
    pub integration: String,
    /// The commit carried there, empty when it already had it so.
    pub carried: String,
    /// Why it could not be carried, or empty.
    pub refused: String,
}

/// What the page asks the easel for in one go.
#[derive(Debug, Clone, Serialize)]
pub struct Reply {
    /// The design system.
    pub design: DesignView,
    /// The catalogue: every base, every custom component.
    pub catalog: Vec<Entry>,
    /// The generations under way or done since the view started.
    pub jobs: Vec<Job>,
    /// How many run at once.
    pub sessions: usize,
    /// The model they run on.
    pub model: String,
    /// Something is written in the workspace no commit carries yet.
    pub edited: bool,
    /// Changes with the branch's files: what the page reloads on.
    pub stamp: String,
    /// Nothing generates: no Claude at hand.
    pub no_claude: bool,
}

const BRANCH: &str = kit::DESIGN_BRANCH;

impl KitDesk {
    /// The easel, nothing generating yet.
    #[must_use]
    pub fn new(setup: Setup) -> Self {
        Self {
            setup: Arc::new(setup),
            files: Arc::default(),
            jobs: Arc::default(),
            running: Arc::default(),
            stopping: Arc::default(),
            workers: Arc::default(),
            rewriting: Arc::default(),
        }
    }

    /// The lock of the component `name`'s rewrites, made on first use.
    fn rewrite_lock(&self, name: &str) -> Arc<Mutex<()>> {
        let mut locks = lock(&self.rewriting);
        Arc::clone(locks.entry(name.to_string()).or_default())
    }

    /// Runs `f` on a blocking thread with the mock-ups port.
    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Mockups, &Mutex<()>) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let mockups = Arc::clone(&self.setup.mockups);
        let files = Arc::clone(&self.files);
        tokio::task::spawn_blocking(move || f(mockups.as_ref(), &files))
            .await
            .map_err(|e| format!("the easel's work was dropped: {e}"))?
    }

    /// Makes the branch and its workspace exist.
    fn ensure(mockups: &dyn Mockups, from: &str) -> Result<PathBuf, String> {
        mockups.ensure_branch(BRANCH, from)?;
        mockups.workspace(BRANCH)
    }

    fn text_of(mockups: &dyn Mockups, path: &str) -> Result<Option<String>, String> {
        Ok(mockups
            .file(BRANCH, path)?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn design_of(mockups: &dyn Mockups) -> Result<Design, String> {
        Ok(Self::text_of(mockups, design::DESIGN_JSON)?
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default())
    }

    fn kit_of(mockups: &dyn Mockups) -> Result<Kit, String> {
        Kit::parse(Self::text_of(mockups, design::KIT_JSON)?.as_deref())
    }

    fn write_json<T: Serialize>(
        mockups: &dyn Mockups,
        path: &str,
        value: &T,
    ) -> Result<(), String> {
        let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        mockups.write(BRANCH, path, format!("{text}\n").as_bytes())
    }

    fn view_of(mockups: &dyn Mockups) -> Result<DesignView, String> {
        let design = Self::design_of(mockups)?;
        let tokens_json = Self::text_of(mockups, design::TOKENS_JSON)?.unwrap_or_default();
        let candidate_json = Self::text_of(mockups, design::CANDIDATE_JSON)?.unwrap_or_default();
        let tokens = if tokens_json.is_empty() {
            Vec::new()
        } else {
            kit::tokens_of(&tokens_json)?
        };
        let candidate = if candidate_json.is_empty() {
            Vec::new()
        } else {
            kit::tokens_of(&candidate_json).unwrap_or_default()
        };
        Ok(DesignView {
            missing: kit::missing(&tokens),
            tokens_css: Self::text_of(mockups, design::TOKENS_CSS)?.unwrap_or_default(),
            design,
            tokens,
            tokens_json,
            candidate,
            candidate_json,
        })
    }

    fn catalog_of(mockups: &dyn Mockups, kit: &Kit) -> Vec<Entry> {
        design::catalog(
            kit,
            |name| {
                Self::text_of(mockups, &Kit::fragment_path(name))
                    .ok()
                    .flatten()
            },
            |name| {
                mockups
                    .file(BRANCH, &draft_path(name))
                    .ok()
                    .flatten()
                    .is_some()
            },
        )
    }

    /// Everything the page shows of the easel. The branch and its workspace
    /// are made on the first visit; a component the code stage recoded gets
    /// its pin on the canvas.
    ///
    /// # Errors
    ///
    /// The repository cannot be reached, or the files are not what they
    /// should be.
    pub async fn reply(&self) -> Result<Reply, String> {
        let from = self.setup.integration_branch.clone();
        let pins = Arc::clone(&self.setup.pins);
        let (design, catalog, edited, stamp) = self
            .blocking(move |mockups, files| {
                let _held = lock(files);
                Self::ensure(mockups, &from)?;
                let design = Self::view_of(mockups)?;
                let kit = Self::kit_of(mockups)?;
                Self::drop_recoded_pins(pins.as_ref(), &kit);
                let catalog = Self::catalog_of(mockups, &kit);
                let state = mockups.state(BRANCH)?;
                Ok((design, catalog, state.edited, state.stamp))
            })
            .await?;
        Ok(Reply {
            design,
            catalog,
            jobs: self.jobs(),
            sessions: self.setup.sessions,
            model: self.setup.model.clone(),
            edited,
            stamp,
            no_claude: self.setup.oneshot.is_none(),
        })
    }

    /// A recoded component without its pin on the canvas gets one.
    fn drop_recoded_pins(pins: &dyn NoteStore, kit: &Kit) {
        let Ok(kept) = pins.notes(design::BOARD) else {
            return;
        };
        for component in kit
            .components
            .iter()
            .filter(|c| !c.recoded.trim().is_empty())
        {
            let pin = design::recoded_pin(component, &now());
            if !kept.iter().any(|k| k.note.id == pin.id) {
                let _ = pins.put(design::BOARD, &pin);
            }
        }
    }

    /// Keeps tokens: a file injected, a hand edit, a generation. Before any
    /// validation they are *the* tokens, candidate; once validated, new
    /// tokens wait as a candidate beside them and the validated ones stay
    /// until the human says so.
    fn keep_tokens(
        mockups: &dyn Mockups,
        json: &str,
        source: Source,
        brief: Option<&str>,
    ) -> Result<DesignView, String> {
        let (done, _added) = kit::completed(json)?;
        let mut design = Self::design_of(mockups)?;
        if design.state == DesignState::Validated {
            mockups.write(BRANCH, design::CANDIDATE_JSON, done.as_bytes())?;
            design.candidate = true;
        } else {
            mockups.write(BRANCH, design::TOKENS_JSON, done.as_bytes())?;
            let tokens = kit::tokens_of(&done)?;
            mockups.write(BRANCH, design::TOKENS_CSS, kit::css_of(&tokens).as_bytes())?;
            design.state = DesignState::Candidate;
            design.candidate = false;
        }
        design.source = source;
        if let Some(brief) = brief {
            design.brief = brief.chars().take(design::MAX_BRIEF).collect();
        }
        Self::write_json(mockups, design::DESIGN_JSON, &design)?;
        Self::view_of(mockups)
    }

    /// Injects a tokens file, or keeps a hand edit of the tokens.
    ///
    /// # Errors
    ///
    /// Not a tokens file, or the workspace could not be written.
    pub async fn inject(&self, json: String, edit: bool) -> Result<DesignView, String> {
        let from = self.setup.integration_branch.clone();
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            Self::ensure(mockups, &from)?;
            if edit {
                // A hand edit of the validated tokens changes them in place
                // and cancels the validation: the human validates again.
                let mut design = Self::design_of(mockups)?;
                if design.state == DesignState::Validated {
                    let (done, _) = kit::completed(&json)?;
                    mockups.write(BRANCH, design::TOKENS_JSON, done.as_bytes())?;
                    let tokens = kit::tokens_of(&done)?;
                    mockups.write(BRANCH, design::TOKENS_CSS, kit::css_of(&tokens).as_bytes())?;
                    design.state = DesignState::Candidate;
                    design.validated_at = String::new();
                    Self::write_json(mockups, design::DESIGN_JSON, &design)?;
                    return Self::view_of(mockups);
                }
            }
            let design = Self::design_of(mockups)?;
            let source = if edit && design.source != Source::None {
                design.source
            } else {
                Source::Injected
            };
            Self::keep_tokens(mockups, &json, source, None)
        })
        .await
    }

    /// Keeps an image: one of the moodboard's, the logo, a component's
    /// reference. Answers the path it is kept at, relative to `mock/`.
    ///
    /// # Errors
    ///
    /// Not an image name, too heavy, a full moodboard, or the workspace
    /// could not be written.
    pub async fn keep_asset(
        &self,
        asset: Asset,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<String, String> {
        let name = design::plain_asset_name(name);
        if !design::is_asset_name(&name) {
            return Err(format!("not an image name: {name}"));
        }
        if bytes.is_empty() || bytes.len() > design::MAX_ASSET_BYTES {
            return Err("an image weighs between one byte and eight megabytes".to_string());
        }
        let from = self.setup.integration_branch.clone();
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            Self::ensure(mockups, &from)?;
            match asset {
                Asset::Moodboard => {
                    let mut design = Self::design_of(mockups)?;
                    let rel = format!("moodboard/{name}");
                    if !design.moodboard.contains(&rel) {
                        if design.moodboard.len() >= design::MAX_MOODBOARD {
                            return Err(format!(
                                "the moodboard holds at most {} images",
                                design::MAX_MOODBOARD
                            ));
                        }
                        design.moodboard.push(rel.clone());
                    }
                    let path = format!("design/{rel}");
                    mockups.write(BRANCH, &path, &bytes)?;
                    Self::write_json(mockups, design::DESIGN_JSON, &design)?;
                    Ok(path)
                }
                Asset::Logo => {
                    let mut design = Self::design_of(mockups)?;
                    if !design.logo.is_empty() && design.logo != name {
                        let _ = mockups.remove(BRANCH, &format!("design/{}", design.logo));
                    }
                    design.logo.clone_from(&name);
                    let path = format!("design/{name}");
                    mockups.write(BRANCH, &path, &bytes)?;
                    Self::write_json(mockups, design::DESIGN_JSON, &design)?;
                    Ok(path)
                }
                Asset::Reference { component } => {
                    let mut kit = Self::kit_of(mockups)?;
                    let entry = kit
                        .component_mut(&component)
                        .ok_or_else(|| format!("no component {component}"))?;
                    let ext = name.rsplit('.').next().unwrap_or("png");
                    let rel = format!("{component}.reference.{ext}");
                    entry.reference.clone_from(&rel);
                    let path = format!("kit/{rel}");
                    mockups.write(BRANCH, &path, &bytes)?;
                    Self::write_json(mockups, design::KIT_JSON, &kit)?;
                    Ok(path)
                }
            }
        })
        .await
    }

    /// Removes an image of the moodboard, or the logo.
    ///
    /// # Errors
    ///
    /// The workspace could not be written.
    pub async fn remove_asset(&self, asset: Asset, name: &str) -> Result<DesignView, String> {
        let name = name.to_string();
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            let mut design = Self::design_of(mockups)?;
            match asset {
                Asset::Moodboard => {
                    let rel = name.strip_prefix("moodboard/").unwrap_or(&name).to_string();
                    design
                        .moodboard
                        .retain(|m| *m != format!("moodboard/{rel}"));
                    mockups.remove(BRANCH, &format!("design/moodboard/{rel}"))?;
                }
                Asset::Logo => {
                    if !design.logo.is_empty() {
                        mockups.remove(BRANCH, &format!("design/{}", design.logo))?;
                    }
                    design.logo.clear();
                }
                Asset::Reference { component } => {
                    let mut kit = Self::kit_of(mockups)?;
                    if let Some(entry) = kit.component_mut(&component) {
                        if !entry.reference.is_empty() {
                            mockups.remove(BRANCH, &format!("kit/{}", entry.reference))?;
                        }
                        entry.reference.clear();
                    }
                    Self::write_json(mockups, design::KIT_JSON, &kit)?;
                }
            }
            Self::write_json(mockups, design::DESIGN_JSON, &design)?;
            Self::view_of(mockups)
        })
        .await
    }

    /// Draws the tokens from the moodboard, the logo and the brief, in one
    /// question to Claude that reads the images. The answer is kept as the
    /// tokens (candidate) or, once validated, as a candidate beside them.
    ///
    /// # Errors
    ///
    /// No Claude, no material, Claude failed, or the answer is not a
    /// tokens file.
    pub async fn generate(&self, brief: String) -> Result<DesignView, String> {
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let model = self.setup.model.clone();
        let from = self.setup.integration_branch.clone();
        let brief: String = brief.chars().take(design::MAX_BRIEF).collect();
        self.blocking(move |mockups, files| {
            let (folder, design) = {
                let _held = lock(files);
                let folder = Self::ensure(mockups, &from)?;
                (folder, Self::design_of(mockups)?)
            };
            if design.moodboard.is_empty() && design.logo.is_empty() && brief.trim().is_empty() {
                return Err(
                    "give the generation something: a moodboard, a logo or a brief".to_string(),
                );
            }
            let prompt = design::generate_prompt(&brief, &design.moodboard, &design.logo);
            let dir = folder.join(kit::DESIGN_DIR);
            let answer = oneshot.ask_in(&model, &prompt, &dir)?;
            let (json, _added) = design::tokens_from_answer(&answer)?;
            let _held = lock(files);
            Self::keep_tokens(mockups, &json, Source::Generated, Some(&brief))
        })
        .await
    }

    /// Validates: the candidate, when one waits, takes the tokens' place;
    /// the state is validated. The kit may be generated from here on.
    ///
    /// # Errors
    ///
    /// No tokens yet, or the workspace could not be written.
    pub async fn validate(&self) -> Result<DesignView, String> {
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            let mut design = Self::design_of(mockups)?;
            if design.candidate {
                let candidate = Self::text_of(mockups, design::CANDIDATE_JSON)?
                    .ok_or("the candidate's file is gone")?;
                mockups.write(BRANCH, design::TOKENS_JSON, candidate.as_bytes())?;
                let tokens = kit::tokens_of(&candidate)?;
                mockups.write(BRANCH, design::TOKENS_CSS, kit::css_of(&tokens).as_bytes())?;
                mockups.remove(BRANCH, design::CANDIDATE_JSON)?;
                design.candidate = false;
            } else if design.state == DesignState::None
                || Self::text_of(mockups, design::TOKENS_JSON)?.is_none()
            {
                return Err("nothing to validate: inject or generate the tokens first".to_string());
            }
            design.state = DesignState::Validated;
            design.validated_at = now();
            Self::write_json(mockups, design::DESIGN_JSON, &design)?;
            Self::view_of(mockups)
        })
        .await
    }

    /// Drops the candidate: the validated tokens stay.
    ///
    /// # Errors
    ///
    /// The workspace could not be written.
    pub async fn drop_candidate(&self) -> Result<DesignView, String> {
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            let mut design = Self::design_of(mockups)?;
            mockups.remove(BRANCH, design::CANDIDATE_JSON)?;
            design.candidate = false;
            Self::write_json(mockups, design::DESIGN_JSON, &design)?;
            Self::view_of(mockups)
        })
        .await
    }

    /// Remembers the roadmap issue that installs the design system.
    ///
    /// # Errors
    ///
    /// The workspace could not be written.
    pub async fn set_issue(&self, url: String) -> Result<DesignView, String> {
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            let mut design = Self::design_of(mockups)?;
            design.issue = url;
            Self::write_json(mockups, design::DESIGN_JSON, &design)?;
            Self::view_of(mockups)
        })
        .await
    }

    /// Ticks the bases: the kit lists every ticked base, pending, and
    /// drops the pending ones unticked.
    ///
    /// # Errors
    ///
    /// The workspace could not be written.
    pub async fn set_checked(&self, ids: Vec<String>) -> Result<Vec<Entry>, String> {
        let from = self.setup.integration_branch.clone();
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            Self::ensure(mockups, &from)?;
            let mut kit = Self::kit_of(mockups)?;
            kit.checked = ids
                .into_iter()
                .filter(|id| kit::base(id).is_some())
                .collect();
            kit.settle();
            Self::write_json(mockups, design::KIT_JSON, &kit)?;
            Ok(Self::catalog_of(mockups, &kit))
        })
        .await
    }

    /// Adds a custom component from the human's words, pending. Its name
    /// is made from the title (`kit::name_for`) unless one is given — a
    /// page that already injects it by that name.
    ///
    /// # Errors
    ///
    /// No title to name it from, a name given that is not one, already
    /// there, or the workspace could not be written.
    pub async fn custom(
        &self,
        name: String,
        title: String,
        prompt: String,
    ) -> Result<Created, String> {
        let name = name.trim().to_string();
        let title: String = title.trim().chars().take(80).collect();
        if !name.is_empty() && !kit::is_name(&name) {
            return Err(
                "a component is named like `HeroBanner`: a capital, then letters and digits"
                    .to_string(),
            );
        }
        if name.is_empty() && title.is_empty() {
            return Err(
                "give the component a name, for humans: its code name is made from it".to_string(),
            );
        }
        if prompt.trim().is_empty() {
            return Err(
                "say what the component is: its prompt is what the AI draws it from".to_string(),
            );
        }
        let from = self.setup.integration_branch.clone();
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            Self::ensure(mockups, &from)?;
            let mut kit = Self::kit_of(mockups)?;
            let name = if name.is_empty() {
                let taken: Vec<&str> = kit
                    .components
                    .iter()
                    .map(|c| c.name.as_str())
                    .chain(kit::BASE.iter().map(|b| b.name))
                    .collect();
                kit::name_for(&title, &taken).ok_or_else(|| {
                    format!("\"{title}\" has no letter to name the component from")
                })?
            } else {
                name
            };
            if kit.component(&name).is_some() || kit::BASE.iter().any(|b| b.name == name) {
                return Err(format!("{name} is already in the kit"));
            }
            kit.components.push(Component {
                name: name.clone(),
                title: if title.is_empty() {
                    name.clone()
                } else {
                    title
                },
                prompt: prompt.chars().take(design::MAX_PROMPT).collect(),
                ..Component::default()
            });
            Self::write_json(mockups, design::KIT_JSON, &kit)?;
            Ok(Created {
                name,
                catalog: Self::catalog_of(mockups, &kit),
            })
        })
        .await
    }

    /// Changes a component's prompt (and title), to generate it again.
    ///
    /// # Errors
    ///
    /// No such component, or the workspace could not be written.
    pub async fn reprompt(
        &self,
        name: String,
        title: String,
        prompt: String,
    ) -> Result<Vec<Entry>, String> {
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            let mut kit = Self::kit_of(mockups)?;
            let entry = kit
                .component_mut(&name)
                .ok_or_else(|| format!("no component {name}"))?;
            entry.prompt = prompt.chars().take(design::MAX_PROMPT).collect();
            if !title.trim().is_empty() {
                entry.title = title.chars().take(80).collect();
            }
            Self::write_json(mockups, design::KIT_JSON, &kit)?;
            Ok(Self::catalog_of(mockups, &kit))
        })
        .await
    }

    /// The generations followed since the view started.
    #[must_use]
    pub fn jobs(&self) -> Vec<Job> {
        lock(&self.jobs).clone()
    }

    /// Starts generating: the components named, or every pending and
    /// failed one; up to `sessions` at once, each one question to Claude.
    /// Resumable: what is generated stays, what failed is tried again.
    /// Answers how many were queued.
    ///
    /// # Errors
    ///
    /// No Claude, the design system not validated, or nothing to do.
    pub async fn start_generation(&self, names: Vec<String>) -> Result<usize, String> {
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let from = self.setup.integration_branch.clone();
        let queued = self
            .blocking(move |mockups, files| {
                let _held = lock(files);
                Self::ensure(mockups, &from)?;
                let design = Self::design_of(mockups)?;
                // A candidate not yet validated already has its own
                // `tokens.css` the first time (`keep_tokens`): the kit can
                // be drawn from it so the simulated page means something
                // before the human commits to it, not only after.
                if design.state == DesignState::None {
                    return Err(
                        "generate or inject the tokens first: the kit is styled from them"
                            .to_string(),
                    );
                }
                let mut kit = Self::kit_of(mockups)?;
                kit.settle();
                let names = design::to_generate(&kit, &names);
                for name in &names {
                    if let Some(c) = kit.component_mut(name) {
                        c.state = ComponentState::Pending;
                        c.why.clear();
                    }
                }
                Self::write_json(mockups, design::KIT_JSON, &kit)?;
                Ok(names)
            })
            .await?;
        if queued.is_empty() {
            return Err(
                "nothing to generate: every component is there, or none is ticked".to_string(),
            );
        }
        {
            let mut jobs = lock(&self.jobs);
            jobs.retain(|j| !queued.contains(&j.name));
            jobs.extend(queued.iter().map(|name| Job {
                name: name.clone(),
                state: JobState::Queued,
                why: String::new(),
                started: String::new(),
                ended: String::new(),
            }));
        }
        self.stopping.store(false, Ordering::SeqCst);
        let wanted = self.setup.sessions.max(1);
        let mut spawned = Vec::new();
        while self.running.load(Ordering::SeqCst) < wanted {
            self.running.fetch_add(1, Ordering::SeqCst);
            let desk = self.clone();
            let oneshot = Arc::clone(&oneshot);
            spawned.push(tokio::task::spawn_blocking(move || {
                desk.work(oneshot.as_ref());
            }));
        }
        // The handles kept for the shutdown; the finished ones let go.
        let mut workers = lock(&self.workers);
        workers.retain(|w| !w.is_finished());
        workers.extend(spawned);
        drop(workers);
        Ok(queued.len())
    }

    /// One session's loop: takes the next queued job, generates it, until
    /// none is left or the easel stops.
    fn work(&self, oneshot: &dyn Oneshot) {
        while !self.stopping.load(Ordering::SeqCst) {
            let next = {
                let mut jobs = lock(&self.jobs);
                let taken = jobs
                    .iter_mut()
                    .find(|j| j.state == JobState::Queued)
                    .map(|job| {
                        job.state = JobState::Running;
                        job.started = now();
                        job.name.clone()
                    });
                drop(jobs);
                taken
            };
            let Some(next) = next else {
                break;
            };
            let outcome = self.generate_one(oneshot, &next);
            let mut jobs = lock(&self.jobs);
            if let Some(job) = jobs.iter_mut().find(|j| j.name == next) {
                job.ended = now();
                match &outcome {
                    Ok(()) => job.state = JobState::Done,
                    Err(why) => {
                        job.state = JobState::Failed;
                        job.why.clone_from(why);
                    }
                }
            }
        }
        self.running.fetch_sub(1, Ordering::SeqCst);
    }

    /// Generates one component: the prompt from its base and the tokens,
    /// one question, the fragment written, the kit told.
    fn generate_one(&self, oneshot: &dyn Oneshot, name: &str) -> Result<(), String> {
        let mockups = self.setup.mockups.as_ref();
        let (component, css, folder) = {
            let _held = lock(&self.files);
            let mut kit = Self::kit_of(mockups)?;
            let entry = kit
                .component_mut(name)
                .ok_or_else(|| format!("no component {name}"))?;
            entry.state = ComponentState::Running;
            let component = entry.clone();
            Self::write_json(mockups, design::KIT_JSON, &kit)?;
            let css = Self::text_of(mockups, design::TOKENS_CSS)?.unwrap_or_default();
            (component, css, mockups.workspace(BRANCH)?)
        };
        let base = kit::base(&component.base);
        let prompt = design::component_prompt(&component, base, &css);
        let answer = if component.reference.is_empty() {
            oneshot.ask(&self.setup.model, &prompt)
        } else {
            oneshot.ask_in(&self.setup.model, &prompt, &folder.join(kit::KIT_DIR))
        };
        let written = answer
            .and_then(|a| design::fragment_from_answer(&a))
            .and_then(|fragment| {
                let checks = kit::check(&fragment);
                mockups.write(BRANCH, &Kit::fragment_path(name), fragment.as_bytes())?;
                Ok(checks)
            });
        let _held = lock(&self.files);
        let mut kit = Self::kit_of(mockups)?;
        let entry = kit
            .component_mut(name)
            .ok_or_else(|| format!("no component {name}"))?;
        let outcome = match written {
            Ok(_checks) => {
                entry.state = ComponentState::Generated;
                entry.why.clear();
                entry.generated_at = now();
                Ok(())
            }
            Err(why) => {
                entry.state = ComponentState::Failed;
                entry.why = why.chars().take(500).collect();
                Err(why)
            }
        };
        Self::write_json(mockups, design::KIT_JSON, &kit)?;
        outcome
    }

    /// Applies requests to a component: one question to Claude, the whole
    /// fragment back, kept as the component's **draft** — what the canvas
    /// shows until it is saved as a fork or a replacement, or dropped.
    /// Answers what the rewrite skipped, `(request id, why)`.
    ///
    /// # Errors
    ///
    /// No Claude, no such component or fragment, or Claude failed.
    pub async fn apply(
        &self,
        name: String,
        asked: Vec<Asked>,
    ) -> Result<Vec<(String, String)>, String> {
        let oneshot = self.setup.oneshot.clone().ok_or(NO_CLAUDE)?;
        let model = self.setup.model.clone();
        if asked.is_empty() {
            return Err("no request to apply".to_string());
        }
        let rewrite = self.rewrite_lock(&name);
        self.blocking(move |mockups, files| {
            // Held from the read to the write: a second request on this
            // component waits, then starts from the draft this one wrote.
            let _rewriting = lock(&rewrite);
            let (fragment, css) = {
                let _held = lock(files);
                let fragment = Self::text_of(mockups, &draft_path(&name))?
                    .or(Self::text_of(mockups, &Kit::fragment_path(&name))?)
                    .ok_or_else(|| format!("{name} has no fragment yet: generate it first"))?;
                (
                    fragment,
                    Self::text_of(mockups, design::TOKENS_CSS)?.unwrap_or_default(),
                )
            };
            let prompt = design::rewrite_prompt(&name, &fragment, &css, &asked);
            let answer = oneshot.ask(&model, &prompt)?;
            let rewritten = design::fragment_from_answer(&answer)?;
            let skipped = design::skipped_in(&rewritten);
            let _held = lock(files);
            mockups.write(BRANCH, &draft_path(&name), rewritten.as_bytes())?;
            Ok(skipped)
        })
        .await
    }

    /// Writes a component's draft as the human retouched it — a token
    /// swapped for another on the easel — when the fragment still passes
    /// the kit's check.
    ///
    /// # Errors
    ///
    /// No such component, a fragment empty, too heavy or failing the
    /// check, or the workspace could not be written.
    pub async fn write_draft(&self, name: String, html: String) -> Result<(), String> {
        if html.trim().is_empty() || html.len() > kit::MAX_FRAGMENT {
            return Err(format!(
                "a fragment weighs between one and {} characters",
                kit::MAX_FRAGMENT
            ));
        }
        let broken = kit::check(&html);
        if !broken.is_empty() {
            return Err(format!(
                "the draft fails the kit's check: {}",
                broken.join("; ")
            ));
        }
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            if Self::kit_of(mockups)?.component(&name).is_none() {
                return Err(format!("no component {name}"));
            }
            mockups.write(BRANCH, &draft_path(&name), html.as_bytes())
        })
        .await
    }

    /// Saves a component's draft: in the component's place, or as a fork —
    /// a new component, the origin untouched. Answers the catalogue.
    ///
    /// # Errors
    ///
    /// No draft, a taken name, or the workspace could not be written.
    pub async fn save(&self, name: String, how: Save) -> Result<Vec<Entry>, String> {
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            let draft = Self::text_of(mockups, &draft_path(&name))?
                .ok_or_else(|| format!("{name} has no draft to save"))?;
            let mut kit = Self::kit_of(mockups)?;
            let origin = kit
                .component(&name)
                .cloned()
                .ok_or_else(|| format!("no component {name}"))?;
            match how {
                Save::Replace => {
                    mockups.write(BRANCH, &Kit::fragment_path(&name), draft.as_bytes())?;
                    if let Some(entry) = kit.component_mut(&name) {
                        entry.generated_at = now();
                        entry.state = ComponentState::Generated;
                    }
                }
                Save::Fork { name: wanted } => {
                    let fork = if wanted.trim().is_empty() {
                        design::fork_name(&kit, &name)
                    } else {
                        wanted.trim().to_string()
                    };
                    if !kit::is_name(&fork) {
                        return Err("a fork is named like `HeroBanner2`: a capital, then letters and digits".to_string());
                    }
                    if kit.component(&fork).is_some() {
                        return Err(format!("{fork} is already in the kit"));
                    }
                    mockups.write(BRANCH, &Kit::fragment_path(&fork), draft.as_bytes())?;
                    kit.components.push(Component {
                        name: fork,
                        base: String::new(),
                        title: format!("{}, a variant", if origin.title.is_empty() { &origin.name } else { &origin.title }),
                        what: origin.what.clone(),
                        prompt: origin.prompt,
                        reference: String::new(),
                        state: ComponentState::Generated,
                        why: String::new(),
                        variant_of: name.clone(),
                        recoded: String::new(),
                        generated_at: now(),
                    });
                }
            }
            mockups.remove(BRANCH, &draft_path(&name))?;
            Self::write_json(mockups, design::KIT_JSON, &kit)?;
            Ok(Self::catalog_of(mockups, &kit))
        })
        .await
    }

    /// Drops a component's draft: the canvas shows the component again.
    ///
    /// # Errors
    ///
    /// The workspace could not be written.
    pub async fn drop_draft(&self, name: String) -> Result<(), String> {
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            mockups.remove(BRANCH, &draft_path(&name))
        })
        .await
    }

    /// Removes a component — its fragment, its draft, its entry — when no
    /// active branch uses it.
    ///
    /// # Errors
    ///
    /// Still used somewhere, or the workspace could not be written.
    pub async fn delete(&self, name: String) -> Result<Vec<Entry>, String> {
        let used = self.usages().await?.remove(&name).unwrap_or_default();
        if !used.is_empty() {
            return Err(format!(
                "{name} is still used on {}: {}",
                used.len(),
                used.iter()
                    .take(5)
                    .map(|u| format!("{} ({})", u.what, u.branch))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            let mut kit = Self::kit_of(mockups)?;
            let Some(entry) = kit.component(&name).cloned() else {
                return Err(format!("no component {name}"));
            };
            mockups.remove(BRANCH, &Kit::fragment_path(&name))?;
            mockups.remove(BRANCH, &draft_path(&name))?;
            if !entry.reference.is_empty() {
                let _ = mockups.remove(BRANCH, &format!("kit/{}", entry.reference));
            }
            kit.components.retain(|c| c.name != name);
            if !entry.base.is_empty() {
                kit.checked.retain(|id| *id != entry.base);
            }
            Self::write_json(mockups, design::KIT_JSON, &kit)?;
            Ok(Self::catalog_of(mockups, &kit))
        })
        .await
    }

    /// Who uses each component: the pages and micro-frontends of every
    /// active branch that inject it.
    ///
    /// # Errors
    ///
    /// The repository cannot be read.
    pub async fn usages(&self) -> Result<BTreeMap<String, Vec<Usage>>, String> {
        self.blocking(|mockups, _files| {
            let mut found: BTreeMap<String, Vec<Usage>> = BTreeMap::new();
            let mut note = |name: String, usage: Usage| {
                let list = found.entry(name).or_default();
                if !list.contains(&usage) {
                    list.push(usage);
                }
            };
            for branch in mockups
                .branches()?
                .into_iter()
                .filter(|b| design::is_active_branch(b))
            {
                let Ok(state) = mockups.state(&branch) else {
                    continue;
                };
                if !state.present {
                    continue;
                }
                for page in &state.pages {
                    let Ok(Some(bytes)) = mockups.file(&branch, &page.path) else {
                        continue;
                    };
                    for name in kit::used_in(&String::from_utf8_lossy(&bytes)) {
                        note(
                            name,
                            Usage {
                                branch: branch.clone(),
                                kind: "page",
                                what: page.path.clone(),
                                title: page.title.clone(),
                            },
                        );
                    }
                }
                for component in state.components.iter().filter(|c| !c.fragment.is_empty()) {
                    let Ok(Some(bytes)) = mockups.file(&branch, &component.fragment) else {
                        continue;
                    };
                    for name in kit::used_in(&String::from_utf8_lossy(&bytes)) {
                        note(
                            name,
                            Usage {
                                branch: branch.clone(),
                                kind: "micro-frontend",
                                what: component.name.clone(),
                                title: component.description.clone(),
                            },
                        );
                    }
                }
            }
            Ok(found)
        })
        .await
    }

    /// Commits the workspace on the `design-system` branch and pushes it —
    /// the kit's stylesheet derived first — then carries the design system
    /// onto the integration branch, where every branch in progress finds it
    /// on its next sync: one writer, so no merge ever meets it.
    ///
    /// # Errors
    ///
    /// Nothing changed, or `git` refused the publication. A carry refused
    /// is not an error: the publication stands, and the reply says why.
    pub async fn publish(&self, message: String) -> Result<Published, String> {
        let to = self.setup.integration_branch.clone();
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            Self::derive_stylesheet(mockups)?;
            let commit = mockups.publish(BRANCH, &message)?;
            let short = commit.get(..10).unwrap_or(&commit);
            let note = format!("design: the design system as published on {BRANCH} ({short})");
            Ok(match mockups.carry(&commit, &to, &kit::OWNED, &note) {
                Ok(carried) => Published {
                    commit,
                    integration: to,
                    carried: carried.unwrap_or_default(),
                    refused: String::new(),
                },
                Err(why) => Published {
                    commit,
                    integration: to,
                    carried: String::new(),
                    refused: why,
                },
            })
        })
        .await
    }

    /// Writes `mock/kit/kit.css` from the generated components' fragments,
    /// when it changed.
    fn derive_stylesheet(mockups: &dyn Mockups) -> Result<(), String> {
        let kit = Self::kit_of(mockups)?;
        let mut parts = Vec::new();
        for component in kit
            .components
            .iter()
            .filter(|c| c.state == ComponentState::Generated)
        {
            if let Some(fragment) = Self::text_of(mockups, &Kit::fragment_path(&component.name))? {
                parts.push((component.name.clone(), fragment));
            }
        }
        let css = kit::stylesheet(&parts);
        if Self::text_of(mockups, kit::KIT_CSS)?.as_deref() != Some(css.as_str()) {
            mockups.write(BRANCH, kit::KIT_CSS, css.as_bytes())?;
        }
        Ok(())
    }

    /// Drops the workspace: the branch as committed again.
    ///
    /// # Errors
    ///
    /// The workspace could not be removed.
    pub async fn discard(&self) -> Result<(), String> {
        self.blocking(|mockups, files| {
            let _held = lock(files);
            mockups.discard(BRANCH)
        })
        .await
    }

    /// What the request AI is told of the kit: the generated components,
    /// one line each.
    ///
    /// # Errors
    ///
    /// The repository cannot be read.
    pub async fn summary(&self) -> Result<String, String> {
        self.blocking(|mockups, files| {
            let _held = lock(files);
            Ok(design::kit_summary(&Self::kit_of(mockups)?))
        })
        .await
    }

    /// The kit as a micro-frontend's generation reads it: a line and a
    /// skeleton per generated component, the variables by name — never the
    /// fragments themselves.
    ///
    /// # Errors
    ///
    /// The repository cannot be read.
    pub async fn index(&self) -> Result<KitIndex, String> {
        self.blocking(|mockups, files| {
            let _held = lock(files);
            let kit = Self::kit_of(mockups)?;
            let mut components = Vec::new();
            for c in kit
                .components
                .iter()
                .filter(|c| c.state == ComponentState::Generated)
            {
                if let Some(fragment) = Self::text_of(mockups, &Kit::fragment_path(&c.name))? {
                    components.push((
                        c.name.clone(),
                        kit::index_line(c, &fragment),
                        kit::skeleton(&fragment),
                    ));
                }
            }
            let css = Self::text_of(mockups, design::TOKENS_CSS)?.unwrap_or_default();
            let tokens = css
                .split([';', '{', '\n'])
                .filter_map(|decl| decl.trim().split_once(':'))
                .map(|(name, _)| name.trim())
                .filter(|name| name.starts_with("--"))
                .map(str::to_string)
                .collect();
            Ok(KitIndex {
                validated: Self::design_of(mockups)?.state == DesignState::Validated,
                components,
                tokens,
            })
        })
        .await
    }

    /// The component a human's name stands for, whatever its state — so
    /// what is asked twice is made once.
    ///
    /// # Errors
    ///
    /// The repository cannot be read.
    pub async fn titled(&self, title: String) -> Result<Option<String>, String> {
        let title = title.trim().to_lowercase();
        self.blocking(move |mockups, files| {
            let _held = lock(files);
            Ok(Self::kit_of(mockups)?
                .components
                .iter()
                .find(|c| c.title.trim().to_lowercase() == title)
                .map(|c| c.name.clone()))
        })
        .await
    }

    /// Where a component's generation stands, when one was asked.
    #[must_use]
    pub fn job_of(&self, name: &str) -> Option<(JobState, String)> {
        lock(&self.jobs)
            .iter()
            .find(|j| j.name == name)
            .map(|j| (j.state, j.why.clone()))
    }

    /// Stops the generations: the sessions finish the component they are
    /// on and take no other. The view is going away.
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        for worker in lock(&self.workers).drain(..) {
            worker.abort();
        }
    }
}

/// Why nothing generates.
const NO_CLAUDE: &str = "no Claude at hand: the view found no `claude` to ask";

/// A component's draft, relative to `mock/`.
fn draft_path(name: &str) -> String {
    format!("kit/{name}.draft.html")
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::adapters::mockups::DirMockups;
    use crate::domain::annotations::Note;
    use crate::ports::KeptNote;

    /// A store in memory.
    #[derive(Default)]
    pub struct Shelf {
        notes: Mutex<Vec<(String, KeptNote)>>,
    }

    // A test fake: each method holds its lock for the whole of its few lines.
    #[allow(clippy::significant_drop_tightening)]
    impl NoteStore for Shelf {
        fn boards(&self) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
        fn notes(&self, board: &str) -> Result<Vec<KeptNote>, String> {
            Ok(lock(&self.notes)
                .iter()
                .filter(|(b, _)| b == board)
                .map(|(_, k)| k.clone())
                .collect())
        }
        fn put(&self, board: &str, note: &Note) -> Result<(), String> {
            lock(&self.notes).push((
                board.to_string(),
                KeptNote {
                    note: note.clone(),
                    deleted_at: None,
                },
            ));
            Ok(())
        }
        fn mark_deleted(&self, _board: &str, _id: &str, _at: Option<&str>) -> Result<(), String> {
            Ok(())
        }
        fn destroy(&self, _board: &str, _id: &str) -> Result<(), String> {
            Ok(())
        }
        fn conversation(&self, _board: &str, _id: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn save_conversation(&self, _b: &str, _i: &str, _j: &str, _t: usize) -> Result<(), String> {
            Ok(())
        }
    }

    /// Claude, answering a fragment made of the prompt's component name.
    struct Styling;

    impl Oneshot for Styling {
        fn ask(&self, _model: &str, prompt: &str) -> Result<String, String> {
            if prompt.contains("`Failing`") {
                return Err("the model refused".to_string());
            }
            if prompt.contains("design tokens") {
                return Ok(kit::default_tokens_json());
            }
            let name = prompt.split('`').nth(1).unwrap_or("X").to_string();
            Ok(format!(
                "```html\n<div class=\"kit-{}\" style=\"color:var(--color-text, #111)\">{name}</div>\n```",
                name.to_ascii_lowercase()
            ))
        }
    }

    fn desk(oneshot: bool) -> (KitDesk, PathBuf) {
        static NTH: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "harness-kit-desk-{}-{}",
            std::process::id(),
            NTH.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("main/mock/pages")).expect("temp dir");
        std::fs::create_dir_all(dir.join("design-system/mock")).expect("temp dir");
        std::fs::write(
            dir.join("main/mock/pages.json"),
            r#"{"version":1,"pages":[{"path":"pages/home.html","title":"Home","objective":"o"}]}"#,
        )
        .expect("manifest");
        std::fs::write(
            dir.join("main/mock/pages/home.html"),
            "<main><div data-kit=\"Button\"></div></main>",
        )
        .expect("page");
        let desk = KitDesk::new(Setup {
            mockups: Arc::new(DirMockups::new(&dir)),
            oneshot: oneshot.then(|| Arc::new(Styling) as Arc<dyn Oneshot>),
            model: "m".into(),
            sessions: 2,
            pins: Arc::new(Shelf::default()),
            integration_branch: "main".into(),
        });
        (desk, dir)
    }

    // One story, end to end: the easel's states are what the test reads.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn the_easel_is_born_empty_then_validates_tokens_and_generates_the_kit() {
        let (desk, dir) = desk(true);
        let reply = desk.reply().await.expect("reply");
        assert_eq!(reply.design.design.state, DesignState::None);
        assert_eq!(reply.catalog.len(), kit::BASE.len());
        assert!(
            desk.start_generation(vec![]).await.is_err(),
            "not validated"
        );
        let view = desk
            .inject(kit::default_tokens_json(), false)
            .await
            .expect("injected");
        assert_eq!(view.design.state, DesignState::Candidate);
        assert_eq!(view.design.source, Source::Injected);
        assert!(view.tokens_css.contains("--color-primary"));
        let view = desk.validate().await.expect("validated");
        assert_eq!(view.design.state, DesignState::Validated);
        // Generated again once validated: a candidate waits beside.
        let view = desk.generate("warm".into()).await.expect("generated");
        assert!(view.design.candidate);
        assert_eq!(view.design.source, Source::Generated);
        assert_eq!(view.design.brief, "warm");
        let view = desk.drop_candidate().await.expect("dropped");
        assert!(!view.design.candidate);
        desk.set_checked(vec!["button".into(), "card".into()])
            .await
            .expect("checked");
        let created = desk
            .custom("Hero".into(), "Hero".into(), "a banner".into())
            .await
            .expect("custom");
        assert!(
            created
                .catalog
                .iter()
                .any(|e| e.name == "Hero" && e.base.is_empty())
        );
        // Only a title: the name is made from it.
        let named = desk
            .custom(
                String::new(),
                "Bannière du héros".into(),
                "another banner".into(),
            )
            .await
            .expect("named from the title");
        assert_eq!(named.name, "BanniereDuHeros");
        assert!(
            desk.custom(String::new(), "  ".into(), "x".into())
                .await
                .is_err()
        );
        assert!(
            desk.custom("hero".into(), String::new(), "x".into())
                .await
                .is_err()
        );
        desk.custom("Failing".into(), String::new(), "fails".into())
            .await
            .expect("custom");
        let queued = desk.start_generation(vec![]).await.expect("started");
        assert_eq!(queued, 5);
        // Wait for the workers.
        for _ in 0..200 {
            if desk
                .jobs()
                .iter()
                .all(|j| matches!(j.state, JobState::Done | JobState::Failed))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let reply = desk.reply().await.expect("reply");
        let hero = reply
            .catalog
            .iter()
            .find(|e| e.name == "Hero")
            .expect("hero");
        assert_eq!(hero.state, ComponentState::Generated);
        assert!(hero.fragment && hero.checks.is_empty(), "{:?}", hero.checks);
        let failing = reply
            .catalog
            .iter()
            .find(|e| e.name == "Failing")
            .expect("failing");
        assert_eq!(failing.state, ComponentState::Failed);
        assert!(failing.why.contains("refused"));
        assert!(reply.jobs.iter().any(|j| j.state == JobState::Failed));
        // Resumable: only the failed one is queued again.
        assert_eq!(desk.start_generation(vec![]).await.expect("again"), 1);
        assert!(desk.summary().await.expect("summary").contains("- Hero"));
        // A draft from requests, saved as a fork, then as a replacement.
        let skipped = desk
            .apply(
                "Hero".into(),
                vec![Asked {
                    id: "r1".into(),
                    text: "bigger".into(),
                    target: String::new(),
                }],
            )
            .await
            .expect("applied");
        assert!(skipped.is_empty());
        // A draft retouched on the easel: kept when it passes the check, refused otherwise.
        assert!(
            desk.write_draft(
                "Hero".into(),
                "<style>.kit-hero { color: #ff0000; }</style><div class=\"kit-hero\">x</div>"
                    .into()
            )
            .await
            .is_err()
        );
        desk.write_draft(
            "Hero".into(),
            "<style>.kit-hero { color: var(--color-text, #111111); }</style><div class=\"kit-hero\">x</div>".into(),
        )
        .await
        .expect("a draft that passes the check");
        assert!(desk.write_draft("Nobody".into(), "<style>.kit-x { color: var(--color-text, #111111); }</style><div class=\"kit-x\">x</div>".into()).await.is_err());
        let entries = desk
            .save(
                "Hero".into(),
                Save::Fork {
                    name: String::new(),
                },
            )
            .await
            .expect("forked");
        let fork = entries.iter().find(|e| e.name == "Hero2").expect("fork");
        assert_eq!(fork.variant_of, "Hero");
        assert!(
            !entries
                .iter()
                .find(|e| e.name == "Hero")
                .expect("hero")
                .draft
        );
        desk.apply(
            "Hero".into(),
            vec![Asked {
                id: "r2".into(),
                text: "x".into(),
                target: String::new(),
            }],
        )
        .await
        .expect("applied");
        desk.save("Hero".into(), Save::Replace)
            .await
            .expect("replaced");
        // Usages: the sample's page uses Button; Hero2 is unused and goes.
        let usages = desk.usages().await.expect("usages");
        assert_eq!(usages.get("Button").map(Vec::len), Some(1));
        assert!(desk.delete("Button".into()).await.is_err(), "still used");
        let entries = desk.delete("Hero2".into()).await.expect("deleted");
        assert!(!entries.iter().any(|e| e.name == "Hero2"));
        // An image, kept and removed.
        let path = desk
            .keep_asset(Asset::Moodboard, "Mood 1.PNG", vec![1, 2, 3])
            .await
            .expect("kept");
        assert_eq!(path, "design/moodboard/mood-1.png");
        assert!(
            desk.keep_asset(Asset::Logo, "x.exe", vec![1])
                .await
                .is_err()
        );
        let view = desk
            .remove_asset(Asset::Moodboard, "moodboard/mood-1.png")
            .await
            .expect("removed");
        assert!(view.design.moodboard.is_empty());
        assert!(
            desk.publish(String::new()).await.is_err(),
            "the sample has no repository"
        );
        desk.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Claude, rewriting slowly: the fragment it is handed, with a mark of
    /// each request it was asked.
    struct Appending;

    impl Oneshot for Appending {
        fn ask(&self, _model: &str, prompt: &str) -> Result<String, String> {
            std::thread::sleep(std::time::Duration::from_millis(80));
            let fragment = prompt
                .split("<fragment>\n")
                .nth(1)
                .and_then(|rest| rest.split("\n</fragment>").next())
                .unwrap_or_default();
            let ids: Vec<&str> = prompt
                .lines()
                .filter_map(|l| l.strip_prefix("- request `"))
                .filter_map(|l| l.split('`').next())
                .collect();
            Ok(format!("{}\n<i>{}</i>", fragment.trim(), ids.join(",")))
        }
    }

    #[tokio::test]
    async fn two_requests_applied_at_once_on_one_component_both_land() {
        let (_, dir) = desk(false);
        let desk = KitDesk::new(Setup {
            mockups: Arc::new(DirMockups::new(&dir)),
            oneshot: Some(Arc::new(Appending)),
            model: "m".into(),
            sessions: 1,
            pins: Arc::new(Shelf::default()),
            integration_branch: "main".into(),
        });
        std::fs::create_dir_all(dir.join("design-system/mock/kit")).expect("kit dir");
        std::fs::write(
            dir.join("design-system/mock/kit/Button.html"),
            "<div class=\"kit-button\">b</div>",
        )
        .expect("fragment");
        let asked = |id: &str| {
            vec![Asked {
                id: id.into(),
                text: format!("change {id}"),
                target: String::new(),
            }]
        };
        let (first, second) = tokio::join!(
            desk.apply("Button".into(), asked("r1")),
            desk.apply("Button".into(), asked("r2"))
        );
        first.expect("first applied");
        second.expect("second applied");
        let draft = std::fs::read_to_string(dir.join("design-system/mock/kit/Button.draft.html"))
            .expect("a draft");
        assert!(
            draft.contains("<i>r1</i>") && draft.contains("<i>r2</i>"),
            "the second started from the first's draft: {draft}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_hand_edit_of_validated_tokens_cancels_the_validation() {
        let (desk, dir) = desk(false);
        desk.inject(kit::default_tokens_json(), false)
            .await
            .expect("injected");
        let view = desk.validate().await.expect("validated");
        assert_eq!(view.design.state, DesignState::Validated);
        assert!(!view.design.validated_at.is_empty());
        let view = desk
            .inject(kit::default_tokens_json(), true)
            .await
            .expect("edited");
        assert_eq!(view.design.state, DesignState::Candidate);
        assert!(view.design.validated_at.is_empty());
        let view = desk.validate().await.expect("validated again");
        assert_eq!(view.design.state, DesignState::Validated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn without_claude_nothing_generates_but_the_rest_works() {
        let (desk, dir) = desk(false);
        let reply = desk.reply().await.expect("reply");
        assert!(reply.no_claude);
        desk.inject(kit::default_tokens_json(), false)
            .await
            .expect("injected");
        desk.validate().await.expect("validated");
        assert!(desk.generate("x".into()).await.is_err());
        assert!(desk.start_generation(vec![]).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
