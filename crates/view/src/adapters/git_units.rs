//! The product's architecture manifests, read with `git` on every branch.
//!
//! From GitHub: a bare mirror the view keeps for itself under
//! `.llocal/view-mirror/`, cloned once and fetched at most once a minute —
//! no run reads it, and the janitor counts it among the stores. From a
//! folder (`--data-dir`): that repository's own branches, never fetched.
//! Under `--demo`: the sample's folders, one per branch.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::ports::{FileDates, RefUnits, UnitFile, Units, UnitsReading};

/// The manifests the list is drawn from, by file name.
const NAMES: [&str; 5] = [
    "capability.json",
    "data-capability.json",
    "micro-ui.json",
    "aggregate.json",
    "concept.json",
];

/// Folders a manifest is never read from: build outputs, dependencies.
const SKIPPED: [&str; 4] = ["node_modules", "target", ".git", "dist"];

/// The branch the data archive lives on: no unit there.
const ARCHIVE: &str = "harness/data-model";

/// A persisted query registry: which Capability reads which fields.
const REGISTRY: &str = "queries/registry.json";

fn wanted(path: &str) -> bool {
    let named = path
        .rsplit('/')
        .next()
        .is_some_and(|name| NAMES.contains(&name))
        || path == REGISTRY
        || path.ends_with(&format!("/{REGISTRY}"));
    named && !path.split('/').any(|p| SKIPPED.contains(&p))
}

/// Where the repository is, and whether it is fetched.
enum Repo {
    /// A bare mirror of `url`, at `dir`.
    Mirror { url: String, dir: PathBuf },
    /// A checkout the human named.
    Local(PathBuf),
}

/// The product's repository as the view reads it with `git`: a bare mirror
/// it keeps and fetches, or a checkout the human named. Shared by what reads
/// the manifests and what reads the mock-up, so one fetch serves both.
pub struct Mirror {
    repo: Repo,
    fresh: Duration,
    fetched: Mutex<Option<Instant>>,
}

impl Mirror {
    /// A bare mirror of `url` kept at `dir`, fetched when older than `fresh`.
    #[must_use]
    pub fn bare(url: &str, dir: &Path, fresh: Duration) -> Self {
        Self {
            repo: Repo::Mirror {
                url: url.to_string(),
                dir: dir.to_path_buf(),
            },
            fresh,
            fetched: Mutex::new(None),
        }
    }

    /// The checkout at `dir`, as it is — never fetched.
    #[must_use]
    pub fn local(dir: &Path) -> Self {
        Self {
            repo: Repo::Local(dir.to_path_buf()),
            fresh: Duration::MAX,
            fetched: Mutex::new(None),
        }
    }

    /// `git <args>` on the repository, its stdout.
    ///
    /// # Errors
    ///
    /// `git` failed, with what it said.
    pub fn git(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let mut command = Command::new("git");
        match &self.repo {
            Repo::Mirror { dir, .. } => command.arg("--git-dir").arg(dir),
            Repo::Local(dir) => command.arg("-C").arg(dir),
        };
        run(command.args(args), input)
    }

    /// The text of `path` on `branch`, `None` when the file is not there.
    ///
    /// # Errors
    ///
    /// `git` failed for another reason than absence.
    pub fn show(&self, branch: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
        let spec = format!("refs/heads/{branch}:{path}");
        match self.git(&["cat-file", "-p", &spec], None) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(why)
                if why.contains("does not exist")
                    || why.contains("ot a valid object name")
                    || why.contains("nvalid object name")
                    || why.contains("exists on disk, but not in") =>
            {
                Ok(None)
            }
            Err(why) => Err(why),
        }
    }

    /// The commit `branch` points at, `None` for a branch that is not there.
    ///
    /// # Errors
    ///
    /// `git` failed for another reason than absence.
    pub fn head_of(&self, branch: &str) -> Result<Option<String>, String> {
        if !self.branches()?.iter().any(|b| b == branch) {
            return Ok(None);
        }
        let reference = format!("refs/heads/{branch}");
        let out = self.git(&["rev-parse", "--verify", "--quiet", &reference], None)?;
        Ok(Some(String::from_utf8_lossy(&out).trim().to_string()))
    }

    /// Clones the mirror the first time, fetches it when stale.
    ///
    /// # Errors
    ///
    /// The clone or the fetch failed.
    pub fn refresh(&self) -> Result<(), String> {
        let Repo::Mirror { url, dir } = &self.repo else {
            return Ok(());
        };
        let last = *self.fetched.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| at.elapsed() < self.fresh) {
            return Ok(());
        }
        if !dir.join("HEAD").is_file() {
            if let Some(parent) = dir.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            let _ = fs::remove_dir_all(dir);
            run(
                Command::new("git")
                    .args(["clone", "--bare", "--quiet", url])
                    .arg(dir),
                None,
            )?;
        }
        self.git(
            &[
                "fetch",
                "--quiet",
                "--prune",
                "origin",
                "+refs/heads/*:refs/heads/*",
            ],
            None,
        )?;
        *self.fetched.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        Ok(())
    }

    /// Every branch but the data archive's.
    ///
    /// # Errors
    ///
    /// `git` failed.
    pub fn branches(&self) -> Result<Vec<String>, String> {
        let out = self.git(
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
            None,
        )?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .map(str::trim)
            .filter(|b| !b.is_empty() && *b != ARCHIVE)
            .map(str::to_string)
            .collect())
    }
}

/// `git`, on the product's repository: the manifests on every branch.
pub struct GitUnits {
    mirror: Arc<Mirror>,
}

impl GitUnits {
    /// Reads the manifests through `mirror`.
    #[must_use]
    pub const fn new(mirror: Arc<Mirror>) -> Self {
        Self { mirror }
    }

    fn git(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
        self.mirror.git(args, input)
    }

    /// `(path, blob)` of every manifest on `branch`.
    fn manifests(&self, branch: &str) -> Result<Vec<(String, String)>, String> {
        let reference = format!("refs/heads/{branch}");
        let out = self.git(&["ls-tree", "-r", "--full-tree", &reference], None)?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|line| {
                let (meta, path) = line.split_once('\t')?;
                let blob = meta.split_whitespace().nth(2)?;
                wanted(path).then(|| (path.to_string(), blob.to_string()))
            })
            .collect())
    }

    /// Every blob's text, in one `git cat-file --batch`.
    fn texts(&self, blobs: &BTreeSet<String>) -> Result<BTreeMap<String, String>, String> {
        if blobs.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut input = String::new();
        for blob in blobs {
            input.push_str(blob);
            input.push('\n');
        }
        let out = self.git(&["cat-file", "--batch"], Some(input.as_bytes()))?;
        Ok(parse_batch(&out))
    }

    fn dates(&self) -> Result<Vec<FileDates>, String> {
        let out = self.git(
            &[
                "log",
                "--all",
                "--format=%x01%cI",
                "--name-only",
                "--",
                ":(glob)**/capability.json",
                ":(glob)**/data-capability.json",
            ],
            None,
        )?;
        Ok(parse_dates(&String::from_utf8_lossy(&out)))
    }
}

impl Units for GitUnits {
    fn read(&self) -> Result<UnitsReading, String> {
        self.mirror.refresh()?;
        let mut listed = Vec::new();
        for branch in self.mirror.branches()? {
            listed.push((branch.clone(), self.manifests(&branch)?));
        }
        let blobs: BTreeSet<String> = listed
            .iter()
            .flat_map(|(_, files)| files.iter().map(|(_, blob)| blob.clone()))
            .collect();
        let texts = self.texts(&blobs)?;
        let refs = listed
            .into_iter()
            .map(|(git_ref, files)| RefUnits {
                git_ref,
                files: files
                    .into_iter()
                    .filter_map(|(path, blob)| {
                        texts.get(&blob).map(|text| UnitFile {
                            path,
                            text: text.clone(),
                        })
                    })
                    .collect(),
            })
            .collect();
        Ok(UnitsReading {
            refs,
            dates: self.dates()?,
        })
    }
}

/// Runs `command` to its end, `input` on its stdin, and answers its stdout —
/// or what it said on stderr when it failed.
///
/// # Errors
///
/// The command could not start, or exited with a failure.
pub fn run(command: &mut Command, input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let shown = format!("{command:?}");
    let mut child = command
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git cannot start: {e}"))?;
    // Written from a thread: a long input fills the pipe while git's
    // output fills the other one.
    let writer = input.and_then(|bytes| {
        let mut stdin = child.stdin.take()?;
        let bytes = bytes.to_vec();
        Some(std::thread::spawn(move || stdin.write_all(&bytes)))
    });
    let out = child
        .wait_with_output()
        .map_err(|e| format!("git did not finish: {e}"))?;
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "{shown}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// `git cat-file --batch`: `<sha> blob <size>\n<bytes>\n`, or
/// `<sha> missing\n`.
pub fn parse_batch(out: &[u8]) -> BTreeMap<String, String> {
    let mut texts = BTreeMap::new();
    let mut at = 0;
    while at < out.len() {
        let Some(end) = out[at..].iter().position(|b| *b == b'\n') else {
            break;
        };
        let header = String::from_utf8_lossy(&out[at..at + end]).to_string();
        at += end + 1;
        let mut parts = header.split_whitespace();
        let (Some(sha), Some(kind), Some(size)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let Ok(size) = size.parse::<usize>() else {
            continue;
        };
        let stop = at.saturating_add(size).min(out.len());
        if kind == "blob" {
            texts.insert(
                sha.to_string(),
                String::from_utf8_lossy(&out[at..stop]).to_string(),
            );
        }
        at = stop.saturating_add(1);
    }
    texts
}

/// `git log --format=%x01%cI --name-only`, newest first: the first date a
/// path is seen with is its last change, the last its creation.
fn parse_dates(log: &str) -> Vec<FileDates> {
    let mut seen: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut date = String::new();
    for line in log.lines() {
        if let Some(stamp) = line.strip_prefix('\u{1}') {
            date = stamp.trim().to_string();
        } else if !line.trim().is_empty() && !date.is_empty() {
            seen.entry(line.trim().to_string())
                .and_modify(|(created, _)| created.clone_from(&date))
                .or_insert_with(|| (date.clone(), date.clone()));
        }
    }
    seen.into_iter()
        .map(|(path, (created, modified))| FileDates {
            path,
            created,
            modified,
        })
        .collect()
}

/// The demo's units: one folder per branch under `dir` (a `/` in a branch
/// name is a `__` in the folder's), and an optional `dates.tsv` —
/// `path`, `created`, `modified`.
pub struct DirUnits {
    dir: PathBuf,
}

impl DirUnits {
    /// The sample at `dir`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }
}

fn walk(root: &Path, at: &Path, files: &mut Vec<UnitFile>) {
    let Ok(entries) = fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, files);
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if wanted(&rel)
            && let Ok(text) = fs::read_to_string(&path)
        {
            files.push(UnitFile { path: rel, text });
        }
    }
}

impl Units for DirUnits {
    fn read(&self) -> Result<UnitsReading, String> {
        let entries =
            fs::read_dir(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let mut refs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let mut files = Vec::new();
            walk(&path, &path, &mut files);
            files.sort_by(|a, b| a.path.cmp(&b.path));
            refs.push(RefUnits {
                git_ref: entry.file_name().to_string_lossy().replace("__", "/"),
                files,
            });
        }
        let dates = fs::read_to_string(self.dir.join("dates.tsv"))
            .map(|text| {
                text.lines()
                    .filter_map(|line| {
                        let mut cells = line.split('\t');
                        Some(FileDates {
                            path: cells.next()?.to_string(),
                            created: cells.next()?.to_string(),
                            modified: cells.next()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(UnitsReading { refs, dates })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_are_known_by_name_outside_build_folders() {
        assert!(wanted("crates/a/capabilities/x/capability.json"));
        assert!(wanted("capability.json"));
        assert!(wanted("apps/a/b/micro-ui.json"));
        assert!(!wanted("apps/node_modules/x/capability.json"));
        assert!(!wanted("crates/a/capability.json.bak"));
        assert!(wanted("crates/campagne/queries/registry.json"));
        assert!(!wanted("crates/campagne/registry.json"));
        assert!(!wanted("node_modules/x/queries/registry.json"));
    }

    #[test]
    fn a_batch_reads_back_every_blob_and_skips_the_missing() {
        let out = b"abc blob 5\nhello\ndef missing\nghi blob 2\n{}\n";
        let texts = parse_batch(out);
        assert_eq!(texts.get("abc").map(String::as_str), Some("hello"));
        assert_eq!(texts.get("ghi").map(String::as_str), Some("{}"));
        assert!(!texts.contains_key("def"));
    }

    #[test]
    fn the_newest_commit_is_the_change_and_the_oldest_the_creation() {
        let log = "\u{1}2026-09-03T10:00:00Z\n\na/capability.json\n\u{1}2026-09-02T10:00:00Z\n\nb/capability.json\n\u{1}2026-09-01T10:00:00Z\n\na/capability.json\n";
        let dates = parse_dates(log);
        let a = dates
            .iter()
            .find(|d| d.path == "a/capability.json")
            .expect("a dated");
        assert_eq!(a.created, "2026-09-01T10:00:00Z");
        assert_eq!(a.modified, "2026-09-03T10:00:00Z");
        assert_eq!(dates.len(), 2);
    }

    /// The real thing, against a throwaway repository: a manifest on `main`,
    /// another on a branch, both dated.
    #[test]
    fn a_local_repository_is_read_branch_by_branch() {
        let dir = std::env::temp_dir().join(format!("harness-view-units-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("crates/c/capabilities/x")).expect("dir");
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .expect("git runs")
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        fs::write(
            dir.join("crates/c/capabilities/x/capability.json"),
            "{\"capability\":\"c.x\"}",
        )
        .expect("write");
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "x"]);
        git(&["checkout", "-q", "-b", "feat/y"]);
        fs::create_dir_all(dir.join("crates/c/capabilities/y")).expect("dir");
        fs::write(
            dir.join("crates/c/capabilities/y/capability.json"),
            "{\"capability\":\"c.y\"}",
        )
        .expect("write");
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "y"]);

        let reading = GitUnits::new(Arc::new(Mirror::local(&dir)))
            .read()
            .expect("read");
        let _ = fs::remove_dir_all(&dir);
        let main = reading
            .refs
            .iter()
            .find(|r| r.git_ref == "main")
            .expect("main");
        let feat = reading
            .refs
            .iter()
            .find(|r| r.git_ref == "feat/y")
            .expect("feat/y");
        assert_eq!(main.files.len(), 1);
        assert_eq!(feat.files.len(), 2);
        assert!(feat.files.iter().any(|f| f.text.contains("c.y")));
        assert_eq!(reading.dates.len(), 2);
    }
}
