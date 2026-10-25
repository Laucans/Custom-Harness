//! The catch-up across the legacy handler split, in the harness's own
//! checkout: a milestone branch still on `mock/src/{commands,reads}.js`
//! taking an integration branch that split them into `mock/handlers/`.
//!
//! git sees the receiving side's changes to a file the head deleted
//! (`UD`): no script solves that. So the receiving side takes the split
//! first, the way the head did — the splitter is deterministic, two trees
//! with the same legacy files split the same — and merges again. What is
//! left are the handler files both sides wrote since the merge base
//! (`AA`): the merge base held them inside a legacy file, never as a file,
//! so git has no common ancestor for them. Their ancestor is the merge
//! base split the same way: each merges three ways on it with
//! `git merge-file`, and only what that leaves conflicted is residual.
//!
//! All in the one checkout, in sequence, never a worktree, on a detached
//! `HEAD` throughout: abort the merge; split the merge base on a detached
//! checkout, committed nowhere a branch keeps; split the branch's tip and
//! commit the split files alone
//! (`mock_handlers::SPLIT_STAGED`) — the splitter's inputs, taken from the
//! head for the run, are put back before the commit; merge the head again.
//! `branching::regenerate` calls it; the generated files are its own loop.

use std::path::Path;
use std::time::Duration;

use harness_architecture::mock_handlers::{
    self, SPLIT_STAGED, SPLITTER, SPLITTER_INPUTS, SPLITTER_TOO_OLD,
};
use harness_core::domain::Halt;

use crate::common::branching::{Checkout, Unmerged, done};

/// How long the splitter may run: it splits, then runs the mock server
/// twice for its self-check.
const SPLIT_TIMEOUT: Duration = Duration::from_mins(5);

/// Where the three sides of a handler file are written for `git
/// merge-file`: under `.git/`, so no `add -A` of the tree ever stages them.
const MERGE_SCRATCH: &str = ".git/harness-merge";

/// Whether the merge met the head's split: a legacy handler file the
/// receiving side changed and the head deleted (`UD`).
pub fn split_needed(conflicted: &[Unmerged]) -> bool {
    conflicted
        .iter()
        .any(|file| file.code == "UD" && mock_handlers::is_legacy(&file.path))
}

/// The handler files both sides added since the merge base (`AA` under
/// `mock/handlers/`): the split wrote them on both.
pub fn split_add_adds(conflicted: &[Unmerged]) -> Vec<String> {
    conflicted
        .iter()
        .filter(|file| file.both_added() && mock_handlers::is_handler_file(&file.path))
        .map(|file| file.path.clone())
        .collect()
}

/// What one `git merge-file` came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeFile {
    /// Merged: its standard output is the file.
    Clean,
    /// Merged with conflict markers: a hand's.
    Conflicts,
    /// An error: no merge at all.
    Failed,
}

/// The verdict of `git merge-file`'s exit code: `0` clean, the number of
/// conflicts (capped at 127) otherwise, anything else an error — a
/// negative exit, a signal.
pub const fn merge_file_verdict(code: Option<i32>) -> MergeFile {
    match code {
        Some(0) => MergeFile::Clean,
        Some(1..=127) => MergeFile::Conflicts,
        _ => MergeFile::Failed,
    }
}

/// What the receiving side's split came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiving {
    /// The branch's split is committed on top of it; the merge base split
    /// the same way is that commit (`Ok`), or why it could not be (`Err`):
    /// every add/add then stays a hand's.
    Split {
        /// The split merge base, or why there is none.
        base: Result<String, String>,
    },
    /// The splitter refused the branch's tree: why (its last line). The
    /// merge is aborted, nothing committed that the next reset keeps.
    Refused(String),
}

/// What one run of the splitter came to.
enum Splitter {
    /// The tree is split.
    Split,
    /// It refused, writing nothing: why.
    Refused(String),
}

/// Takes the head's split on the receiving side of a merge that met it
/// ([`split_needed`]): the merge aborted, the merge base of `branch`'s tip
/// and `head` split on a detached checkout and committed there, then
/// `branch`'s tip (the detached `HEAD` the call found) checked out again,
/// split and the split committed on it — the
/// splitter's inputs taken from `head` for each run, never committed. The
/// caller merges `head` again.
///
/// # Errors
/// A git step or a write that fails, the splitter that cannot start or is
/// refused for its node (the toolchain's fault): why.
pub async fn split_receiving(
    checkout: &Checkout,
    branch: &str,
    integration: &str,
    head: &str,
) -> Result<Receiving, String> {
    let repo = &checkout.repo;
    done("aborting the merge", repo.abort_merge().await)?;
    // The branch's tip, checked out detached: the catch-up never commits on
    // a local branch before its push.
    let tip = repo
        .head_sha()
        .await
        .map_err(|halt| halt.reason().to_string())?;
    let base = match repo
        .merge_base("HEAD", head)
        .await
        .map_err(|halt| halt.reason().to_string())?
    {
        Some(merge_base) => split_base(checkout, &merge_base, head).await?,
        None => Err(format!("{branch} and {head} have no merge base")),
    };
    done("checkout", repo.checkout(&tip, true).await)?;
    done("clean", repo.clean().await)?;
    if let Splitter::Refused(why) = run_splitter(checkout, head).await? {
        return Ok(Receiving::Refused(why));
    }
    let staged: Vec<String> = SPLIT_STAGED
        .iter()
        .map(|path| (*path).to_string())
        .collect();
    done("staging the split", repo.stage_paths(&staged).await)?;
    done(
        "committing the split",
        repo.commit(&format!(
            "chore(mock): split the legacy handler files into mock/handlers/<id>.js before \
             taking {integration} ({})",
            head.get(..7).unwrap_or(head)
        ))
        .await,
    )?;
    // The inputs taken from the head go back to the branch's own: the
    // merge brings the head's.
    done(
        "putting the splitter's inputs back",
        repo.reset_hard("HEAD").await,
    )?;
    done("clean", repo.clean().await)?;
    Ok(Receiving::Split { base })
}

/// The merge base `merge_base` split on a detached checkout and committed
/// there: the commit, or why the splitter refused it.
async fn split_base(
    checkout: &Checkout,
    merge_base: &str,
    head: &str,
) -> Result<Result<String, String>, String> {
    let repo = &checkout.repo;
    done(
        "checking the merge base out",
        repo.checkout(merge_base, true).await,
    )?;
    if let Splitter::Refused(why) = run_splitter(checkout, head).await? {
        return Ok(Err(format!("the merge base cannot be split: {why}")));
    }
    done("staging the split merge base", repo.stage_all().await)?;
    done(
        "committing the split merge base",
        repo.commit(&format!("split of the merge base {merge_base}"))
            .await,
    )?;
    repo.head_sha()
        .await
        .map(Ok)
        .map_err(|halt| halt.reason().to_string())
}

/// Runs the splitter on the tree checked out, its inputs written from
/// `head` over the tree's — on the disk only, never staged.
async fn run_splitter(checkout: &Checkout, head: &str) -> Result<Splitter, String> {
    for input in SPLITTER_INPUTS {
        let Some(text) = checkout
            .repo
            .show(&format!("{head}:{input}"))
            .await
            .map_err(|halt| halt.reason().to_string())?
        else {
            return Ok(Splitter::Refused(format!("{head} has no {input}")));
        };
        write(checkout, Path::new(input), &text)?;
    }
    let ran = checkout
        .process
        .run(
            "node",
            &[SPLITTER.to_string()],
            &checkout.root,
            SPLIT_TIMEOUT,
        )
        .await
        .map_err(|halt| format!("node {SPLITTER} cannot start: {}", halt.reason()))?;
    if ran.ok() {
        return Ok(Splitter::Split);
    }
    if ran.code.is_none() || ran.stderr.contains(SPLITTER_TOO_OLD) {
        return Err(format!("node {SPLITTER} failed: {}", ran.why()));
    }
    Ok(Splitter::Refused(ran.why()))
}

/// Writes `text` at `path` under the checkout's root, its folder made.
fn write(checkout: &Checkout, path: &Path, text: &str) -> Result<(), String> {
    let at = checkout.root.join(path);
    let cannot = |halt: Halt| format!("{} cannot be written: {}", path.display(), halt.reason());
    if let Some(folder) = at.parent() {
        checkout.disk.create_dir_all(folder).map_err(cannot)?;
    }
    checkout.disk.write_to_string(&at, text).map_err(cannot)
}

/// Merges each add/add handler file of `paths` three ways — ours (stage 2)
/// and theirs (stage 3) on `split_base`'s, an empty file where the split
/// merge base has none — and writes the clean ones into the tree: those
/// merged. A conflicted merge leaves its path as it is.
///
/// # Errors
/// A read or a write that fails, or `git merge-file` erring: why.
pub async fn merge_on_split_base(
    checkout: &Checkout,
    split_base: &str,
    paths: &[String],
) -> Result<Vec<String>, String> {
    let repo = &checkout.repo;
    let read = async |spec: String| {
        repo.show(&spec)
            .await
            .map_err(|halt| format!("{spec} cannot be read: {}", halt.reason()))
    };
    let scratch = Path::new(MERGE_SCRATCH);
    let sides = [
        scratch.join("ours"),
        scratch.join("base"),
        scratch.join("theirs"),
    ];
    let mut merged = Vec::new();
    for path in paths {
        let (Some(ours), Some(theirs)) = (
            read(format!(":2:{path}")).await?,
            read(format!(":3:{path}")).await?,
        ) else {
            continue;
        };
        let base = read(format!("{split_base}:{path}"))
            .await?
            .unwrap_or_default();
        for (side, text) in sides.iter().zip([&ours, &base, &theirs]) {
            write(checkout, side, text)?;
        }
        let [ours_at, base_at, theirs_at] = sides.clone().map(|side| checkout.root.join(side));
        let ran = repo
            .merge_file(&ours_at, &base_at, &theirs_at)
            .await
            .map_err(|halt| format!("git merge-file on {path} failed: {}", halt.reason()))?;
        match merge_file_verdict(ran.code) {
            MergeFile::Clean => {
                write(checkout, Path::new(path), &ran.stdout)?;
                merged.push(path.clone());
            }
            MergeFile::Conflicts => {}
            MergeFile::Failed => {
                return Err(format!("git merge-file on {path} failed: {}", ran.why()));
            }
        }
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unmerged(code: &str, path: &str) -> Unmerged {
        Unmerged {
            path: path.to_string(),
            code: code.to_string(),
        }
    }

    #[test]
    fn a_split_is_needed_for_a_legacy_file_we_changed_and_the_head_deleted_only() {
        assert!(split_needed(&[
            unmerged("UU", "Cargo.lock"),
            unmerged("UD", "mock/src/reads.js"),
        ]));
        assert!(split_needed(&[unmerged("UD", "mock/src/commands.js")]));
        for (code, path) in [
            ("DU", "mock/src/commands.js"),
            ("UU", "mock/src/commands.js"),
            ("DD", "mock/src/reads.js"),
            ("UD", "mock/src/server.js"),
            ("UD", "crates/x/src/lib.rs"),
        ] {
            assert!(!split_needed(&[unmerged(code, path)]), "{code} {path}");
        }
        assert!(!split_needed(&[]));
    }

    #[test]
    fn the_add_adds_are_the_handler_files_both_sides_added() {
        assert_eq!(
            split_add_adds(&[
                unmerged("AA", "mock/handlers/_lib/commands.js"),
                unmerged("UU", "mock/handlers/campaign.list.js"),
                unmerged("AA", "mock/handlers/README.md"),
                unmerged("AA", "crates/a/Cargo.toml"),
                unmerged("AA", "mock/handlers/campaign.saveEntity.js"),
            ]),
            [
                "mock/handlers/_lib/commands.js",
                "mock/handlers/campaign.saveEntity.js"
            ]
        );
    }

    proptest::proptest! {
        #[test]
        fn a_merge_file_exit_is_clean_conflicted_or_an_error(code in proptest::option::of(proptest::num::i32::ANY)) {
            let expected = match code {
                Some(0) => MergeFile::Clean,
                Some(n) if (1..=127).contains(&n) => MergeFile::Conflicts,
                _ => MergeFile::Failed,
            };
            proptest::prop_assert_eq!(merge_file_verdict(code), expected);
        }
    }

    #[test]
    fn the_exit_codes_at_the_edges_of_a_conflict_count() {
        assert_eq!(merge_file_verdict(Some(0)), MergeFile::Clean);
        assert_eq!(merge_file_verdict(Some(1)), MergeFile::Conflicts);
        assert_eq!(merge_file_verdict(Some(127)), MergeFile::Conflicts);
        assert_eq!(merge_file_verdict(Some(128)), MergeFile::Failed);
        assert_eq!(merge_file_verdict(Some(-1)), MergeFile::Failed);
        assert_eq!(merge_file_verdict(None), MergeFile::Failed);
    }
}
