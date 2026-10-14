//! A request reworked with the AI in the files: the branch's mock-up as it
//! was when the session opened — every file under `mock/` — kept until the
//! session closes. Validated, what changed goes in the mock-up's history
//! and in the pin's step (its Undo); dropped, every file is put back.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::domain::mock_history::Facet;
use crate::ports::Mockups;

use super::AppState;
use super::mock_history::{self, Gesture};

/// The reworks open, by pin (`<board> <id>`).
pub type Reworks = Arc<Mutex<HashMap<String, Snapshot>>>;

/// The branch's mock-up as a rework found it.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// The branch of the workspace.
    pub branch: String,
    /// Every file under `mock/`, by path, with its bytes.
    files: Vec<(String, Vec<u8>)>,
}

/// A file a rework changed: as it was and as it is (`None`, absent).
type Change = (String, Option<Vec<u8>>, Option<Vec<u8>>);

fn key(board: &str, pin: &str) -> String {
    format!("{board} {pin}")
}

/// Every file of the branch's mock-up, with its bytes.
async fn read_all(
    mockups: Arc<dyn Mockups>,
    branch: String,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    tokio::task::spawn_blocking(move || {
        let mut all = Vec::new();
        for f in mockups.files(&branch)? {
            if let Some(bytes) = mockups.file(&branch, &f.path)? {
                all.push((f.path, bytes));
            }
        }
        Ok(all)
    })
    .await
    .map_err(|e| format!("the read was dropped: {e}"))?
}

/// The mock-up as a rework of this pin finds it, kept — once: a rework
/// opened again goes on from the first one's.
///
/// # Errors
///
/// The workspace could not be read.
pub(super) async fn take(
    state: &AppState,
    branch: &str,
    board: &str,
    pin: &str,
) -> Result<(), String> {
    let mockups = state.mockups.clone().ok_or("nothing reads the mock-up")?;
    if lock(&state.reworks).contains_key(&key(board, pin)) {
        return Ok(());
    }
    let files = read_all(mockups, branch.to_string()).await?;
    lock(&state.reworks)
        .entry(key(board, pin))
        .or_insert_with(|| Snapshot {
            branch: branch.to_string(),
            files,
        });
    Ok(())
}

/// What changed since the snapshot: each file as it was and as it is.
#[must_use]
pub fn changes(before: &[(String, Vec<u8>)], after: &[(String, Vec<u8>)]) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    for (path, was) in before {
        let now = after
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| b.clone());
        if now.as_ref() != Some(was) {
            out.push((path.clone(), Some(was.clone()), now));
        }
    }
    for (path, now) in after {
        if !before.iter().any(|(p, _)| p == path) {
            out.push((path.clone(), None, Some(now.clone())));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A rework's session closed: validated, what it changed is kept in the
/// history and in the pin's step; dropped, every file is put back. No
/// rework open for the pin: nothing to do. Answers how many files changed.
///
/// # Errors
///
/// The workspace could not be read or written back.
pub(super) async fn settle(
    state: &AppState,
    board: &str,
    pin: &str,
    keep: bool,
) -> Result<usize, String> {
    let Some(snapshot) = lock(&state.reworks).remove(&key(board, pin)) else {
        return Ok(0);
    };
    let mockups = state.mockups.clone().ok_or("nothing reads the mock-up")?;
    let now = read_all(Arc::clone(&mockups), snapshot.branch.clone()).await?;
    let changed = changes(&snapshot.files, &now);
    let count = changed.len();
    if keep {
        // The history and the pin's step keep text: a file not text is kept
        // as it is now, out of reach of an undo.
        let text = |b: &Option<Vec<u8>>| b.as_ref().and_then(|b| String::from_utf8(b.clone()).ok());
        let readable: Vec<&Change> = changed
            .iter()
            .filter(|(_, was, now)| {
                was.as_ref().is_none_or(|b| std::str::from_utf8(b).is_ok())
                    && now.as_ref().is_none_or(|b| std::str::from_utf8(b).is_ok())
            })
            .collect();
        let befores: Vec<(String, Option<String>)> = readable
            .iter()
            .map(|(p, was, _)| (p.clone(), text(was)))
            .collect();
        super::mockup::keep_steps(state, &snapshot.branch, &[pin.to_string()], board, &befores)?;
        let gesture = Gesture::new(&snapshot.branch, "a request reworked with the AI");
        mock_history::record(
            state,
            &gesture,
            readable
                .iter()
                .map(|(path, was, now)| Facet::File {
                    path: path.clone(),
                    before: text(was),
                    after: text(now),
                })
                .collect(),
        );
        return Ok(count);
    }
    let branch = snapshot.branch.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        for (path, was, _) in changed {
            match was {
                Some(bytes) => mockups.write(&branch, &path, &bytes)?,
                None => mockups.remove(&branch, &path)?,
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("the put back was dropped: {e}"))??;
    Ok(count)
}

fn lock(reworks: &Reworks) -> std::sync::MutexGuard<'_, HashMap<String, Snapshot>> {
    reworks.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change as a test reads it: the path, before, after.
    type Said<'a> = (&'a str, Option<&'a [u8]>, Option<&'a [u8]>);

    #[test]
    fn a_rework_s_changes_are_the_files_it_wrote_added_or_removed() {
        let file = |p: &str, b: &str| (p.to_string(), b.as_bytes().to_vec());
        let before = [file("a", "1"), file("b", "2"), file("c", "3")];
        let after = [file("a", "1"), file("b", "two"), file("d", "4")];
        let got = changes(&before, &after);
        let said: Vec<Said<'_>> = got
            .iter()
            .map(|(p, w, n)| (p.as_str(), w.as_deref(), n.as_deref()))
            .collect();
        assert_eq!(
            said,
            [
                ("b", Some(&b"2"[..]), Some(&b"two"[..])),
                ("c", Some(&b"3"[..]), None),
                ("d", None, Some(&b"4"[..])),
            ]
        );
        assert!(changes(&before, &before).is_empty());
    }
}
