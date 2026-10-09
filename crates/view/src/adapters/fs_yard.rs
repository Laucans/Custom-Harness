//! The yard on disk: `.llocal/`, walked with `std::fs` and swept with it.
//!
//! A walk follows no symlink — a link is weighed as itself, never as what it
//! points at — and a removal unlinks a link rather than emptying its target.
//! Every path the page names is checked to stay inside the yard before
//! anything is touched.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::domain::cleanup::{BUILD_OUTPUTS, DEPENDENCIES, LOGS, WORKSPACES};
use crate::ports::{Heap, RunHeap, Survey, WorkspaceHeap, Yard};

/// The janitor's settings, beside the stores.
const SETTINGS: &str = "janitor.json";

/// How deep a workspace is searched for caches: a monorepo keeps its
/// `node_modules` a few levels down, never twenty.
const CACHE_DEPTH: usize = 6;

/// The `.llocal/` of one checkout.
pub struct FsYard {
    root: PathBuf,
}

impl FsYard {
    /// The yard under `state_root` — `<state_root>/.llocal`.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            root: state_root.join(".llocal"),
        }
    }

    /// `rel` under the yard, or why not: no `..`, no root, nothing empty.
    fn inside(&self, rel: &str) -> Result<PathBuf, String> {
        let path = Path::new(rel);
        let plain = !rel.is_empty()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if plain {
            Ok(self.root.join(path))
        } else {
            Err(format!("{rel}: not a path inside the yard"))
        }
    }

    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn heap(&self, path: &Path, now: SystemTime) -> Heap {
        let (bytes, newest) = weigh(path);
        let dir = fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
        Heap {
            rel: self.rel(path),
            bytes,
            idle_secs: idle(newest, now),
            dir,
        }
    }

    fn workspace(&self, path: &Path, now: SystemTime) -> WorkspaceHeap {
        let mut found = Vec::new();
        caches_in(path, 0, &mut found);
        WorkspaceHeap {
            heap: self.heap(path, now),
            git: path.join(".git").exists(),
            caches: found.iter().map(|cache| self.heap(cache, now)).collect(),
        }
    }

    fn runs(&self, now: SystemTime) -> Vec<RunHeap> {
        let mut runs = Vec::new();
        for workflow in children(&self.root.join(LOGS)) {
            if !is_dir(&workflow) {
                continue;
            }
            let name = file_name(&workflow);
            for run in children(&workflow) {
                if !is_dir(&run) {
                    continue;
                }
                let id = file_name(&run);
                let flow = workflow.join(format!("flow-{id}.jsonl"));
                runs.push(RunHeap {
                    workflow: name.clone(),
                    run: id,
                    heap: self.heap(&run, now),
                    flow: flow.is_file().then(|| self.heap(&flow, now)),
                });
            }
        }
        runs
    }
}

impl Yard for FsYard {
    fn survey(&self) -> Survey {
        let now = SystemTime::now();
        let folders: Vec<Heap> = children(&self.root)
            .iter()
            .map(|path| self.heap(path, now))
            .collect();
        let workspaces = children(&self.root.join(WORKSPACES))
            .iter()
            .filter(|path| is_dir(path))
            .map(|path| self.workspace(path, now))
            .collect();
        Survey {
            taken_at: jiff::Timestamp::now()
                .strftime("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
            total: folders.iter().map(|heap| heap.bytes).sum(),
            folders,
            workspaces,
            runs: self.runs(now),
        }
    }

    fn remove(&self, rel: &str) -> Result<u64, String> {
        let path = self.inside(rel)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("{rel}: {e}"))?;
        let (bytes, _) = weigh(&path);
        let done = if meta.is_dir() {
            // `remove_dir_all` unlinks the symlinks it meets, never follows.
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        done.map(|()| bytes).map_err(|e| format!("{rel}: {e}"))
    }

    fn read_settings(&self) -> Option<String> {
        fs::read_to_string(self.root.join(SETTINGS)).ok()
    }

    fn write_settings(&self, json: &str) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        let path = self.root.join(SETTINGS);
        fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The entries of a folder, sorted; none when it cannot be read.
fn children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    paths
}

fn is_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The cache folders under `dir`, not searched inside a cache or `.git`.
fn caches_in(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > CACHE_DEPTH {
        return;
    }
    for child in children(dir) {
        if !is_dir(&child) {
            continue;
        }
        let name = file_name(&child);
        if name == ".git" {
            continue;
        }
        if BUILD_OUTPUTS.contains(&name.as_str()) || DEPENDENCIES.contains(&name.as_str()) {
            found.push(child);
        } else {
            caches_in(&child, depth + 1, found);
        }
    }
}

/// The bytes under `path` as the disk holds them, and its newest write.
/// A symlink counts as itself; a folder that cannot be read counts as empty.
fn weigh(path: &Path) -> (u64, Option<SystemTime>) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, None);
    };
    let mut bytes = on_disk(&meta);
    let mut newest = meta.modified().ok();
    if meta.is_dir() {
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                bytes = bytes.saturating_add(on_disk(&meta));
                if let Ok(at) = meta.modified() {
                    newest = Some(newest.map_or(at, |n| n.max(at)));
                }
                // `DirEntry::metadata` does not follow a symlink, so a link
                // to a folder is never a folder here.
                if meta.is_dir() {
                    stack.push(entry.path());
                }
            }
        }
    }
    (bytes, newest)
}

/// What a file takes on the disk: its blocks, not its length — a sparse
/// file frees what it holds, not what it claims.
#[cfg(unix)]
fn on_disk(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    meta.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn on_disk(meta: &fs::Metadata) -> u64 {
    meta.len()
}

/// Seconds since `newest`; a heap with no readable clock counts as fresh,
/// so it is never swept for want of one.
fn idle(newest: Option<SystemTime>, now: SystemTime) -> u64 {
    newest
        .and_then(|at| now.duration_since(at).ok())
        .map_or(0, |gap| gap.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "harness-yard-{name}-{}-{}",
                std::process::id(),
                jiff::Timestamp::now().as_nanosecond()
            ));
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }

        fn write(&self, rel: &str, bytes: usize) {
            let path = self.0.join(".llocal").join(rel);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(path, vec![b'x'; bytes]).expect("write");
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_survey_finds_the_clones_their_caches_and_the_runs() {
        let dir = Dir::new("survey");
        dir.write("agentic_workspaces/lane-0/.git/HEAD", 10);
        dir.write("agentic_workspaces/lane-0/src/main.rs", 10);
        dir.write("agentic_workspaces/lane-0/target/debug/app", 9_000);
        dir.write(
            "agentic_workspaces/lane-0/web/node_modules/a/index.js",
            5_000,
        );
        dir.write("agentic_workspaces/lane-0/.git/target/not-a-cache", 10);
        dir.write("logs/split/20261008-120000/run.log", 100);
        dir.write("logs/split/flow-20261008-120000.jsonl", 100);
        dir.write("logs/split/costs.tsv", 10);
        dir.write("harness.db", 10);
        let yard = FsYard::new(&dir.0);
        let survey = yard.survey();

        let names: Vec<&str> = survey.folders.iter().map(|h| h.rel.as_str()).collect();
        assert_eq!(names, ["agentic_workspaces", "harness.db", "logs"]);
        assert!(survey.total >= 14_000, "{}", survey.total);

        assert_eq!(survey.workspaces.len(), 1);
        let lane = &survey.workspaces[0];
        assert!(lane.git);
        let caches: Vec<&str> = lane.caches.iter().map(|h| h.rel.as_str()).collect();
        assert_eq!(
            caches,
            [
                "agentic_workspaces/lane-0/target",
                "agentic_workspaces/lane-0/web/node_modules"
            ],
            "nothing searched inside .git"
        );
        assert!(lane.caches[0].bytes >= 9_000);
        assert!(lane.caches[0].idle_secs < 60, "just written");

        assert_eq!(survey.runs.len(), 1);
        let run = &survey.runs[0];
        assert_eq!(
            (run.workflow.as_str(), run.run.as_str()),
            ("split", "20261008-120000")
        );
        assert_eq!(
            run.flow.as_ref().map(|f| f.rel.as_str()),
            Some("logs/split/flow-20261008-120000.jsonl")
        );
    }

    #[test]
    fn a_removal_stays_in_the_yard_and_says_what_it_freed() {
        let dir = Dir::new("remove");
        dir.write("agentic_workspaces/lane-0/target/debug/app", 9_000);
        let yard = FsYard::new(&dir.0);
        let freed = yard
            .remove("agentic_workspaces/lane-0/target")
            .expect("removed");
        assert!(freed >= 9_000);
        assert!(
            !dir.0
                .join(".llocal/agentic_workspaces/lane-0/target")
                .exists()
        );
        assert!(dir.0.join(".llocal/agentic_workspaces/lane-0").exists());
        for bad in ["", "../outside", "/etc", "logs/../../x", "./logs"] {
            assert!(yard.remove(bad).is_err(), "{bad:?} is refused");
        }
        assert!(yard.remove("agentic_workspaces/gone").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_weighed_and_unlinked_as_itself_never_followed() {
        let dir = Dir::new("link");
        let outside = Dir::new("link-target");
        fs::write(outside.0.join("precious"), vec![b'x'; 50_000]).expect("write");
        dir.write("agentic_workspaces/lane-0/keep", 1);
        std::os::unix::fs::symlink(
            &outside.0,
            dir.0.join(".llocal/agentic_workspaces/lane-0/target"),
        )
        .expect("symlink");
        let yard = FsYard::new(&dir.0);
        let survey = yard.survey();
        assert!(
            survey.total < 50_000,
            "the target is not weighed: {}",
            survey.total
        );
        yard.remove("agentic_workspaces/lane-0/target")
            .expect("unlinked");
        assert!(
            outside.0.join("precious").exists(),
            "the target is untouched"
        );
    }

    #[test]
    fn settings_round_trip_beside_the_stores() {
        let dir = Dir::new("settings");
        let yard = FsYard::new(&dir.0);
        assert!(yard.read_settings().is_none());
        yard.write_settings("{\"auto_sweep\":true}")
            .expect("written");
        assert_eq!(
            yard.read_settings().as_deref(),
            Some("{\"auto_sweep\":true}")
        );
    }
}
