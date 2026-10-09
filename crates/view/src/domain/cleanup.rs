//! The janitor's rules: what in the yard is no longer useful, and how heavy a
//! human allowed the yard to be.
//!
//! Static, all of it: a heap is judged by its name, its weight and how long
//! it has been idle — no model reads anything. The walk is the `Yard` port's,
//! the sweeping too; this module only decides, and a test reads the decision
//! off a survey it wrote.
//!
//! What goes, and when:
//!
//! - a **build output** (`target`, `.next`, …) of a workspace nobody wrote to
//!   for two hours — rebuilt by the next run that needs it;
//! - the **dependencies** (`node_modules`, `.venv`, …) of a workspace idle a
//!   week — reinstalled by `harness doctor`, so later than the outputs;
//! - a **run's traces** older than a month, its flow file with them;
//! - the **throwaway clones** of `init/`, a day after the last;
//! - a **stray** — a folder the harness never writes — only when the human
//!   asked for it, and a day after it was last touched.
//!
//! What stays: a workspace's code (a clone is reset, never deleted — a
//! human's `--use-workspace` may name it), the ledgers and stores under
//! `logs/` and the yard itself, the lanes' journals, the locks.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ports::{Heap, Survey, WorkspaceHeap};

/// How often the yard is weighed again without anyone asking.
pub const CHRONIC: Duration = Duration::from_hours(2);

const GIB: u64 = 1024 * 1024 * 1024;

/// What the yard may weigh until a human says otherwise.
pub const DEFAULT_LIMIT: u64 = 50 * GIB;

/// The least a limit may be: under it the yard cannot hold one clone.
pub const LEAST_LIMIT: u64 = GIB;

/// A build output goes once its workspace has been idle this long.
pub const BUILD_IDLE: u64 = 2 * 3600;

/// Dependencies go once their workspace has been idle this long.
pub const DEPENDENCIES_IDLE: u64 = 7 * 86_400;

/// A run's traces go once this old.
pub const RUN_RETENTION: u64 = 30 * 86_400;

/// The throwaway clones of `init/` go once idle this long.
pub const INIT_IDLE: u64 = 86_400;

/// A stray goes — when allowed — once idle this long.
pub const STRAY_IDLE: u64 = 86_400;

/// The folders a build leaves, rebuilt on the next one. Only names no
/// project tracks in git: `dist` or `build` are sometimes committed, and a
/// sweep must never take what a reset would not give back.
pub const BUILD_OUTPUTS: [&str; 6] = [
    "target",
    ".next",
    ".turbo",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
];

/// The folders an install leaves, reinstalled by the doctor when missing.
pub const DEPENDENCIES: [&str; 4] = ["node_modules", ".venv", "venv", ".tox"];

/// The folder of the clones.
pub const WORKSPACES: &str = "agentic_workspaces";

/// The folder of the traces.
pub const LOGS: &str = "logs";

/// The folder of the throwaway clones `harness init-repo` makes.
pub const INIT: &str = "init";

/// Every folder the harness itself writes under the yard — the launcher's
/// own list, mirrored: a store added there without its name here shows up
/// as a stray, which is the reminder to add it.
const KNOWN: [&str; 8] = [
    WORKSPACES,
    LOGS,
    "grill",
    INIT,
    "lanes",
    "planner-locks",
    "split-locks",
    "pr-fix-locks",
];

/// What a human allowed: how heavy the yard may be, and whether the janitor
/// sweeps on their own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// What the yard may weigh.
    pub limit_bytes: u64,
    /// Sweep without asking when the chronic weighing passes the threshold.
    pub auto_sweep: bool,
    /// The threshold, as a share of the limit.
    pub threshold_percent: u8,
    /// Strays — folders the harness never writes — go too.
    pub sweep_strays: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            limit_bytes: DEFAULT_LIMIT,
            auto_sweep: false,
            threshold_percent: 80,
            sweep_strays: false,
        }
    }
}

impl Settings {
    /// Settings as kept on disk, or as the page sends them.
    ///
    /// # Errors
    ///
    /// Not JSON, or out of bounds ([`Settings::check`]).
    pub fn parse(json: &str) -> Result<Self, String> {
        let settings: Self =
            serde_json::from_str(json).map_err(|e| format!("settings unreadable: {e}"))?;
        settings.check()?;
        Ok(settings)
    }

    /// Whether these settings make sense.
    ///
    /// # Errors
    ///
    /// A limit under [`LEAST_LIMIT`], or a threshold outside 1–100 %.
    pub fn check(&self) -> Result<(), String> {
        if self.limit_bytes < LEAST_LIMIT {
            return Err(format!(
                "a limit of {} is under 1 GiB: the yard cannot hold one clone",
                human(self.limit_bytes)
            ));
        }
        if self.threshold_percent == 0 || self.threshold_percent > 100 {
            return Err(format!(
                "a threshold of {} %: between 1 and 100",
                self.threshold_percent
            ));
        }
        Ok(())
    }

    /// The weight past which an automatic sweep starts.
    #[must_use]
    pub fn threshold_bytes(&self) -> u64 {
        self.limit_bytes
            .saturating_mul(u64::from(self.threshold_percent))
            .saturating_div(100)
    }

    /// As kept on disk.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// What a heap is for — one slice of the pie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// A clone's own files: code, `.git`.
    Code,
    /// What a build left.
    Build,
    /// What an install left.
    Dependencies,
    /// The runs' traces and their ledgers.
    Traces,
    /// The throwaway clones of `init/`.
    Init,
    /// A folder the harness never writes.
    Strays,
    /// The stores and journals: the event store, the lanes' logs, the locks.
    Stores,
}

impl Category {
    /// Every category, in the order the pie shows them.
    pub const ALL: [Self; 7] = [
        Self::Code,
        Self::Build,
        Self::Dependencies,
        Self::Traces,
        Self::Init,
        Self::Strays,
        Self::Stores,
    ];
}

/// One slice of the pie: a category's weight, and how much of it would go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Slice {
    /// The category.
    pub category: Category,
    /// Bytes in it.
    pub bytes: u64,
    /// Bytes of it a sweep would free.
    pub sweepable: u64,
}

/// One heap a sweep removes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sweep {
    /// Relative to the yard.
    pub rel: String,
    /// Its weight.
    pub bytes: u64,
    /// Which slice it comes off.
    pub category: Category,
    /// Why it goes, in a sentence.
    pub why: String,
}

/// One heap that would go but stays, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kept {
    /// Relative to the yard.
    pub rel: String,
    /// Its weight.
    pub bytes: u64,
    /// Why it stays.
    pub why: String,
}

/// What the janitor makes of a survey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnosis {
    /// When the survey was made.
    pub at: String,
    /// What the yard weighs.
    pub used: u64,
    /// What it may weigh.
    pub limit: u64,
    /// Past this, an automatic sweep starts.
    pub threshold: u64,
    /// The pie, in [`Category::ALL`] order, empty slices included.
    pub slices: Vec<Slice>,
    /// What a sweep removes, heaviest first.
    pub sweep: Vec<Sweep>,
    /// What stays although it could go, heaviest first.
    pub kept: Vec<Kept>,
    /// What a sweep frees, all told.
    pub freeable: u64,
    /// The yard weighs more than it may.
    pub over_limit: bool,
    /// The yard weighs more than the threshold.
    pub over_threshold: bool,
}

/// One heap a sweep could not remove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failed {
    /// Relative to the yard.
    pub rel: String,
    /// What the yard said.
    pub why: String,
}

/// What a sweep did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// When it ended.
    pub at: String,
    /// Bytes freed.
    pub freed: u64,
    /// What went.
    pub removed: Vec<Sweep>,
    /// What would not go.
    pub failed: Vec<Failed>,
    /// The janitor swept on their own, the chronic weighing having passed the
    /// threshold — rather than on a click.
    pub automatic: bool,
}

/// Judges a survey.
#[must_use]
pub fn diagnose(survey: &Survey, settings: &Settings) -> Diagnosis {
    let mut pie = Pie::default();
    let mut sweep = Vec::new();
    let mut kept = Vec::new();

    for workspace in &survey.workspaces {
        judge_workspace(workspace, &mut pie, &mut sweep, &mut kept);
    }
    for run in &survey.runs {
        if run.heap.idle_secs >= RUN_RETENTION {
            let why = format!("traces of a run {} old", ago(run.heap.idle_secs));
            sweep.push(Sweep {
                rel: run.heap.rel.clone(),
                bytes: run.heap.bytes,
                category: Category::Traces,
                why: why.clone(),
            });
            pie.sweepable(Category::Traces, run.heap.bytes);
            if let Some(flow) = &run.flow {
                sweep.push(Sweep {
                    rel: flow.rel.clone(),
                    bytes: flow.bytes,
                    category: Category::Traces,
                    why,
                });
                pie.sweepable(Category::Traces, flow.bytes);
            }
        }
    }
    for heap in &survey.folders {
        judge_folder(heap, settings, &mut pie, &mut sweep, &mut kept);
    }

    sweep.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.rel.cmp(&b.rel)));
    kept.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.rel.cmp(&b.rel)));
    let freeable = sweep.iter().map(|s| s.bytes).sum();
    let threshold = settings.threshold_bytes();
    Diagnosis {
        at: survey.taken_at.clone(),
        used: survey.total,
        limit: settings.limit_bytes,
        threshold,
        slices: pie.slices(),
        sweep,
        kept,
        freeable,
        over_limit: survey.total > settings.limit_bytes,
        over_threshold: survey.total > threshold,
    }
}

/// A workspace's caches go when the workspace is idle; its code never.
fn judge_workspace(
    workspace: &WorkspaceHeap,
    pie: &mut Pie,
    sweep: &mut Vec<Sweep>,
    kept: &mut Vec<Kept>,
) {
    if !workspace.git {
        // Not a checkout this harness made: provisioning leaves it alone,
        // and so does the janitor — the same rule, both ways.
        pie.add(Category::Strays, workspace.heap.bytes);
        kept.push(Kept {
            rel: workspace.heap.rel.clone(),
            bytes: workspace.heap.bytes,
            why: "under the workspaces but no checkout: not this harness's to touch".to_string(),
        });
        return;
    }
    let cached: u64 = workspace.caches.iter().map(|c| c.bytes).sum();
    pie.add(Category::Code, workspace.heap.bytes.saturating_sub(cached));
    for cache in &workspace.caches {
        let (category, idle, what) = if is_dependencies(&cache.rel) {
            (Category::Dependencies, DEPENDENCIES_IDLE, "dependencies")
        } else {
            (Category::Build, BUILD_IDLE, "build output")
        };
        pie.add(category, cache.bytes);
        let idle_for = cache.idle_secs.min(workspace.heap.idle_secs);
        if idle_for >= idle {
            sweep.push(Sweep {
                rel: cache.rel.clone(),
                bytes: cache.bytes,
                category,
                why: format!("{what} of a workspace idle {}", ago(idle_for)),
            });
            pie.sweepable(category, cache.bytes);
        } else {
            kept.push(Kept {
                rel: cache.rel.clone(),
                bytes: cache.bytes,
                why: format!(
                    "{what} written {} ago — goes after {}",
                    ago(idle_for),
                    ago(idle)
                ),
            });
        }
    }
}

/// A folder directly under the yard: the harness's own are kept or, for
/// `init/`, swept whole; a stray goes only on the human's word.
fn judge_folder(
    heap: &Heap,
    settings: &Settings,
    pie: &mut Pie,
    sweep: &mut Vec<Sweep>,
    kept: &mut Vec<Kept>,
) {
    let name = heap.rel.as_str();
    if !heap.dir {
        // A file at the yard's root is a store — the event store, the janitor's
        // own settings — never a stray.
        pie.add(Category::Stores, heap.bytes);
        return;
    }
    match name {
        WORKSPACES => {} // counted clone by clone
        LOGS => pie.add(Category::Traces, heap.bytes),
        INIT => {
            pie.add(Category::Init, heap.bytes);
            if heap.bytes == 0 {
                return;
            }
            if heap.idle_secs >= INIT_IDLE {
                sweep.push(Sweep {
                    rel: heap.rel.clone(),
                    bytes: heap.bytes,
                    category: Category::Init,
                    why: format!(
                        "throwaway clones of init-repo, idle {}",
                        ago(heap.idle_secs)
                    ),
                });
                pie.sweepable(Category::Init, heap.bytes);
            } else {
                kept.push(Kept {
                    rel: heap.rel.clone(),
                    bytes: heap.bytes,
                    why: format!(
                        "init-repo wrote here {} ago — goes after {}",
                        ago(heap.idle_secs),
                        ago(INIT_IDLE)
                    ),
                });
            }
        }
        known if KNOWN.contains(&known) => pie.add(Category::Stores, heap.bytes),
        _ => {
            pie.add(Category::Strays, heap.bytes);
            if settings.sweep_strays && heap.idle_secs >= STRAY_IDLE {
                sweep.push(Sweep {
                    rel: heap.rel.clone(),
                    bytes: heap.bytes,
                    category: Category::Strays,
                    why: format!(
                        "not a folder this harness writes, idle {} — strays go, as you asked",
                        ago(heap.idle_secs)
                    ),
                });
                pie.sweepable(Category::Strays, heap.bytes);
            } else {
                kept.push(Kept {
                    rel: heap.rel.clone(),
                    bytes: heap.bytes,
                    why: if settings.sweep_strays {
                        format!(
                            "not a folder this harness writes, touched {} ago — goes after {}",
                            ago(heap.idle_secs),
                            ago(STRAY_IDLE)
                        )
                    } else {
                        "not a folder this harness writes — left alone unless you allow strays"
                            .to_string()
                    },
                });
            }
        }
    }
}

/// Whether a cache path ends in a dependencies folder.
fn is_dependencies(rel: &str) -> bool {
    rel.rsplit('/')
        .next()
        .is_some_and(|name| DEPENDENCIES.contains(&name))
}

/// The pie, slice by slice.
#[derive(Default)]
struct Pie {
    bytes: [u64; 7],
    sweepable: [u64; 7],
}

impl Pie {
    fn slot(category: Category) -> usize {
        Category::ALL
            .iter()
            .position(|c| *c == category)
            .unwrap_or_default()
    }

    fn add(&mut self, category: Category, bytes: u64) {
        let at = Self::slot(category);
        self.bytes[at] = self.bytes[at].saturating_add(bytes);
    }

    fn sweepable(&mut self, category: Category, bytes: u64) {
        let at = Self::slot(category);
        self.sweepable[at] = self.sweepable[at].saturating_add(bytes);
    }

    fn slices(&self) -> Vec<Slice> {
        Category::ALL
            .iter()
            .enumerate()
            .map(|(at, category)| Slice {
                category: *category,
                bytes: self.bytes[at],
                sweepable: self.sweepable[at],
            })
            .collect()
    }
}

/// A duration in seconds, in a word: `40 min`, `3 h`, `12 d`.
#[must_use]
pub fn ago(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min", s / 60),
        s if s < 86_400 => format!("{} h", s / 3600),
        s => format!("{} d", s / 86_400),
    }
}

/// Bytes, in a word: `2.3 GB`, `640 MB`.
#[must_use]
pub fn human(bytes: u64) -> String {
    const STEPS: [(&str, u64); 4] = [
        ("GB", 1024 * 1024 * 1024),
        ("MB", 1024 * 1024),
        ("kB", 1024),
        ("B", 1),
    ];
    for (unit, size) in STEPS {
        if bytes >= size {
            // Fits in an f64's mantissa for any disk this century; a lost
            // bit under the units digit of a GB does not show.
            #[allow(clippy::cast_precision_loss)]
            let value = bytes as f64 / size as f64;
            return if size == 1 || value >= 100.0 {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
    }
    "0 B".to_string()
}

#[cfg(test)]
pub mod fake {
    //! An in-memory yard: the survey a test wrote, the removals it asked.

    use std::sync::Mutex;

    use crate::ports::{Survey, Yard};

    /// A yard whose contents are what the test wrote.
    #[derive(Default)]
    pub struct Lot {
        /// What a survey returns.
        pub survey: Survey,
        /// Every removal, in order.
        pub removed: Mutex<Vec<String>>,
        /// A removal the OS refuses.
        pub stuck: Option<String>,
        /// The settings file.
        pub settings: Mutex<Option<String>>,
    }

    impl Lot {
        /// The weight of `rel` in the survey, as the heaps have it.
        fn weight(&self, rel: &str) -> Option<u64> {
            let s = &self.survey;
            s.folders
                .iter()
                .find(|h| h.rel == rel)
                .map(|h| h.bytes)
                .or_else(|| {
                    s.workspaces
                        .iter()
                        .flat_map(|w| w.caches.iter())
                        .find(|h| h.rel == rel)
                        .map(|h| h.bytes)
                })
                .or_else(|| {
                    s.runs.iter().find_map(|r| {
                        if r.heap.rel == rel {
                            Some(r.heap.bytes)
                        } else {
                            r.flow.as_ref().filter(|f| f.rel == rel).map(|f| f.bytes)
                        }
                    })
                })
        }
    }

    impl Yard for Lot {
        fn survey(&self) -> Survey {
            self.survey.clone()
        }

        fn remove(&self, rel: &str) -> Result<u64, String> {
            if self.stuck.as_deref() == Some(rel) {
                return Err("permission denied".to_string());
            }
            let bytes = self
                .weight(rel)
                .ok_or_else(|| format!("{rel}: not in the yard"))?;
            if let Ok(mut removed) = self.removed.lock() {
                removed.push(rel.to_string());
            }
            Ok(bytes)
        }

        fn read_settings(&self) -> Option<String> {
            self.settings.lock().ok().and_then(|s| s.clone())
        }

        fn write_settings(&self, json: &str) -> Result<(), String> {
            if let Ok(mut settings) = self.settings.lock() {
                *settings = Some(json.to_string());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::RunHeap;

    fn heap(rel: &str, bytes: u64, idle_secs: u64) -> Heap {
        Heap {
            rel: rel.to_string(),
            bytes,
            idle_secs,
            dir: true,
        }
    }

    fn workspace(name: &str, idle: u64, caches: Vec<Heap>) -> WorkspaceHeap {
        let weight: u64 = caches.iter().map(|c| c.bytes).sum();
        WorkspaceHeap {
            heap: heap(&format!("{WORKSPACES}/{name}"), weight + 100, idle),
            git: true,
            caches,
        }
    }

    fn survey() -> Survey {
        let lane = workspace(
            "lane-0",
            3 * 3600,
            vec![
                heap("agentic_workspaces/lane-0/target", 2_000, 3 * 3600),
                heap("agentic_workspaces/lane-0/node_modules", 500, 3 * 3600),
            ],
        );
        let busy = workspace(
            "lane-1",
            60,
            vec![heap("agentic_workspaces/lane-1/target", 900, 60)],
        );
        let old = RunHeap {
            workflow: "split".to_string(),
            run: "20260801-120000".to_string(),
            heap: heap("logs/split/20260801-120000", 40, 40 * 86_400),
            flow: Some(Heap {
                rel: "logs/split/flow-20260801-120000.jsonl".to_string(),
                bytes: 2,
                idle_secs: 40 * 86_400,
                dir: false,
            }),
        };
        let fresh = RunHeap {
            workflow: "split".to_string(),
            run: "20261008-120000".to_string(),
            heap: heap("logs/split/20261008-120000", 30, 3600),
            flow: None,
        };
        Survey {
            taken_at: "2026-10-09T10:00:00Z".to_string(),
            total: 10_000,
            folders: vec![
                heap(WORKSPACES, 3_700, 60),
                heap(LOGS, 70, 3600),
                heap(INIT, 300, 2 * 86_400),
                heap("archive", 191, 30 * 86_400),
                heap("lanes", 8, 60),
                Heap {
                    rel: "harness.db".to_string(),
                    bytes: 4,
                    idle_secs: 10,
                    dir: false,
                },
            ],
            workspaces: vec![lane, busy],
            runs: vec![old, fresh],
        }
    }

    fn rels(sweep: &[Sweep]) -> Vec<&str> {
        sweep.iter().map(|s| s.rel.as_str()).collect()
    }

    #[test]
    fn an_idle_workspace_loses_its_build_output_a_busy_one_keeps_it() {
        let d = diagnose(&survey(), &Settings::default());
        let going = rels(&d.sweep);
        assert!(
            going.contains(&"agentic_workspaces/lane-0/target"),
            "{going:?}"
        );
        assert!(
            !going.contains(&"agentic_workspaces/lane-1/target"),
            "written a minute ago: {going:?}"
        );
        let kept: Vec<&str> = d.kept.iter().map(|k| k.rel.as_str()).collect();
        assert!(
            kept.contains(&"agentic_workspaces/lane-1/target"),
            "{kept:?}"
        );
    }

    #[test]
    fn dependencies_wait_a_week_where_a_build_output_waits_two_hours() {
        let d = diagnose(&survey(), &Settings::default());
        let going = rels(&d.sweep);
        assert!(!going.contains(&"agentic_workspaces/lane-0/node_modules"));
        let mut s = survey();
        s.workspaces[0].heap.idle_secs = 8 * 86_400;
        s.workspaces[0].caches[1].idle_secs = 8 * 86_400;
        let going = diagnose(&s, &Settings::default()).sweep;
        assert!(rels(&going).contains(&"agentic_workspaces/lane-0/node_modules"));
        assert!(
            going
                .iter()
                .any(|s| s.category == Category::Dependencies && s.why.contains("8 d"))
        );
    }

    #[test]
    fn an_old_run_goes_with_its_flow_file_a_fresh_one_stays() {
        let d = diagnose(&survey(), &Settings::default());
        let going = rels(&d.sweep);
        assert!(going.contains(&"logs/split/20260801-120000"));
        assert!(going.contains(&"logs/split/flow-20260801-120000.jsonl"));
        assert!(!going.contains(&"logs/split/20261008-120000"));
    }

    #[test]
    fn init_clones_go_after_a_day_and_strays_only_on_the_human_s_word() {
        let d = diagnose(&survey(), &Settings::default());
        let going = rels(&d.sweep);
        assert!(going.contains(&"init"));
        assert!(!going.contains(&"archive"), "{going:?}");
        assert!(
            d.kept
                .iter()
                .any(|k| k.rel == "archive" && k.why.contains("allow strays"))
        );
        let allowed = Settings {
            sweep_strays: true,
            ..Settings::default()
        };
        let d = diagnose(&survey(), &allowed);
        assert!(rels(&d.sweep).contains(&"archive"));
        assert!(
            !rels(&d.sweep).contains(&"lanes"),
            "a known folder is not a stray"
        );
    }

    #[test]
    fn a_workspace_without_git_is_a_stray_left_alone() {
        let mut s = survey();
        s.workspaces.push(WorkspaceHeap {
            heap: heap("agentic_workspaces/notes", 50, 90 * 86_400),
            git: false,
            caches: vec![heap("agentic_workspaces/notes/target", 20, 90 * 86_400)],
        });
        let allowed = Settings {
            sweep_strays: true,
            ..Settings::default()
        };
        let d = diagnose(&s, &allowed);
        assert!(
            !rels(&d.sweep)
                .iter()
                .any(|r| r.starts_with("agentic_workspaces/notes"))
        );
        assert!(d.kept.iter().any(|k| k.rel == "agentic_workspaces/notes"));
    }

    #[test]
    fn the_pie_adds_up_and_says_what_each_slice_would_lose() {
        let d = diagnose(&survey(), &Settings::default());
        let slice = |c: Category| d.slices.iter().find(|s| s.category == c).expect("slice");
        assert_eq!(slice(Category::Build).bytes, 2_900);
        assert_eq!(slice(Category::Build).sweepable, 2_000);
        assert_eq!(slice(Category::Dependencies).bytes, 500);
        assert_eq!(
            slice(Category::Code).bytes,
            200,
            "100 per clone besides the caches"
        );
        assert_eq!(slice(Category::Traces).bytes, 70);
        assert_eq!(slice(Category::Traces).sweepable, 42);
        assert_eq!(slice(Category::Init).bytes, 300);
        assert_eq!(slice(Category::Strays).bytes, 191);
        assert_eq!(
            slice(Category::Stores).bytes,
            12,
            "lanes and the event store"
        );
        assert_eq!(d.freeable, 2_000 + 42 + 300);
        assert_eq!(
            d.sweep[0].rel, "agentic_workspaces/lane-0/target",
            "heaviest first"
        );
        assert_eq!(d.slices.len(), Category::ALL.len());
    }

    #[test]
    fn the_limit_and_the_threshold_are_read_against_the_weight() {
        let settings = Settings {
            limit_bytes: 8 * GIB,
            threshold_percent: 50,
            ..Settings::default()
        };
        let mut s = survey();
        s.total = 5 * GIB;
        let d = diagnose(&s, &settings);
        assert!(d.over_threshold);
        assert!(!d.over_limit);
        assert_eq!(d.threshold, 4 * GIB);
        s.total = 9 * GIB;
        assert!(diagnose(&s, &settings).over_limit);
    }

    #[test]
    fn settings_default_to_fifty_gigabytes_and_refuse_nonsense() {
        let settings = Settings::default();
        assert_eq!(settings.limit_bytes, 50 * GIB);
        assert!(!settings.auto_sweep);
        assert_eq!(settings.threshold_percent, 80);
        let back = Settings::parse(&settings.to_json()).expect("round trip");
        assert_eq!(back, settings);
        assert!(Settings::parse(r#"{"limit_bytes": 5}"#).is_err());
        assert!(Settings::parse(r#"{"threshold_percent": 0}"#).is_err());
        assert!(Settings::parse(r#"{"threshold_percent": 101}"#).is_err());
        assert!(Settings::parse("nope").is_err());
        // A file from an older the janitor, with a field missing, still reads.
        let partial = Settings::parse(r#"{"auto_sweep": true}"#).expect("defaults fill in");
        assert!(partial.auto_sweep);
        assert_eq!(partial.limit_bytes, DEFAULT_LIMIT);
    }

    #[test]
    fn weights_and_durations_read_as_words() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(900), "900 B");
        assert_eq!(human(2 * 1024 * 1024 * 1024 + 300 * 1024 * 1024), "2.3 GB");
        assert_eq!(human(191 * 1024 * 1024), "191 MB");
        assert_eq!(ago(30), "30 s");
        assert_eq!(ago(2_400), "40 min");
        assert_eq!(ago(3 * 3600), "3 h");
        assert_eq!(ago(12 * 86_400), "12 d");
    }
}
