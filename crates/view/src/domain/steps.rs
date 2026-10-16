//! What a request pin did to the mock-up, kept so its Undo takes it back:
//! each file of `mock/` it wrote, as it was before, and when. A pin's step
//! grows from its Propose (the micro-frontend's draft) to its validation
//! (the fragment, the fork, the manifest, the page); a step that wrote one
//! of the same files after it built on it, so an Undo takes it back too.

use serde::{Deserialize, Serialize};

/// A file of the mock-up as it was before a step wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Before {
    /// Its path under `mock/`.
    pub path: String,
    /// What it held; `None`, it was not there.
    pub text: Option<String>,
    /// When it was written, in the branch's order of writes.
    pub seq: u64,
}

/// What one pin did to the mock-up of a branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    /// The pin, by its id.
    pub pin: String,
    /// The board the pin is on.
    pub board: String,
    /// The files it wrote, each as it was before its first write.
    pub files: Vec<Before>,
}

impl Step {
    /// Keeps a file as it was before the step wrote it, at `seq` — once:
    /// the first before is the true one.
    pub fn keep(&mut self, path: &str, text: Option<String>, seq: u64) {
        if !self.files.iter().any(|b| b.path == path) {
            self.files.push(Before {
                path: path.to_string(),
                text,
                seq,
            });
        }
    }

    /// The step wrote a file after `other` wrote it: it built on it.
    #[must_use]
    pub fn builds_on(&self, other: &Self) -> bool {
        self.files.iter().any(|b| {
            other
                .files
                .iter()
                .any(|o| o.path == b.path && b.seq > o.seq)
        })
    }

    /// Its last write.
    #[must_use]
    pub fn last(&self) -> u64 {
        self.files.iter().map(|b| b.seq).max().unwrap_or_default()
    }
}

/// The steps an Undo of `pin` takes back, latest first: its own, and every
/// step that built on one of them. Empty: the pin did nothing kept.
#[must_use]
pub fn undone_with<'a>(steps: &'a [Step], pin: &str) -> Vec<&'a Step> {
    let Some(first) = steps.iter().find(|s| s.pin == pin) else {
        return Vec::new();
    };
    let mut taken: Vec<&Step> = vec![first];
    loop {
        let more: Vec<&Step> = steps
            .iter()
            .filter(|s| !taken.iter().any(|t| t.pin == s.pin))
            .filter(|s| taken.iter().any(|t| s.builds_on(t)))
            .collect();
        if more.is_empty() {
            break;
        }
        taken.extend(more);
    }
    taken.sort_by_key(|s| std::cmp::Reverse(s.last()));
    taken
}

/// The files as they were before the steps undone: for each path, its
/// earliest before among them.
#[must_use]
pub fn restored(undone: &[&Step]) -> Vec<Before> {
    let mut kept: Vec<Before> = Vec::new();
    for b in undone.iter().flat_map(|s| &s.files) {
        match kept.iter_mut().find(|k| k.path == b.path) {
            Some(k) if b.seq < k.seq => *k = b.clone(),
            Some(_) => {}
            None => kept.push(b.clone()),
        }
    }
    kept.sort_by(|a, b| a.path.cmp(&b.path));
    kept
}

/// The number the next write of a branch takes: after every one kept.
#[must_use]
pub fn next_seq(steps: &[Step]) -> u64 {
    steps
        .iter()
        .map(Step::last)
        .max()
        .map_or(1, |m| m.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn step(pin: &str, files: &[(&str, Option<&str>, u64)]) -> Step {
        let mut s = Step {
            pin: pin.to_string(),
            ..Step::default()
        };
        for (path, text, seq) in files {
            s.keep(path, text.map(ToString::to_string), *seq);
        }
        s
    }

    #[test]
    fn a_file_is_kept_as_it_was_before_the_first_write() {
        let mut s = step("a", &[("components/X.draft.html", None, 1)]);
        s.keep(
            "components/X.draft.html",
            Some("written by a".to_string()),
            4,
        );
        assert_eq!(s.files.len(), 1);
        assert_eq!(
            (s.files[0].text.as_deref(), s.files[0].seq),
            (None, 1),
            "the first before is the true one"
        );
    }

    #[test]
    fn an_undo_takes_back_the_steps_built_on_it_and_only_them() {
        let steps = [
            // a drafts X, b drafts X on a's draft, a's validation comes last.
            step(
                "a",
                &[
                    ("components/X.draft.html", None, 1),
                    ("components/X.html", Some("old X"), 6),
                    ("pages.json", Some("after d"), 7),
                ],
            ),
            step("other", &[("components/Y.draft.html", None, 2)]),
            step("b", &[("components/X.draft.html", Some("a's draft"), 3)]),
            // d validated a fork before a's validation: a did not build on it backwards.
            step(
                "d",
                &[
                    ("pages.json", Some("{}"), 4),
                    ("components/Z2.html", None, 5),
                ],
            ),
        ];
        let pins = |pin: &str| -> Vec<String> {
            undone_with(&steps, pin)
                .iter()
                .map(|s| s.pin.clone())
                .collect()
        };
        assert_eq!(pins("a"), ["a", "b"], "b built on a's draft; latest first");
        assert_eq!(
            pins("d"),
            ["a", "d", "b"],
            "a wrote the manifest after d, and b built on a"
        );
        assert_eq!(pins("b"), ["b"], "a step b built on is never taken back");
        assert_eq!(pins("other"), ["other"]);
        let seen = &pins("gone");
        assert!(seen.is_empty(), "{seen:?}");

        let files = restored(&undone_with(&steps, "d"));
        assert_eq!(
            files
                .iter()
                .map(|b| (b.path.as_str(), b.text.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("components/X.draft.html", None),
                ("components/X.html", Some("old X")),
                ("components/Z2.html", None),
                ("pages.json", Some("{}")),
            ],
            "the earliest before of each file"
        );
        assert_eq!(next_seq(&steps), 8);
        assert_eq!(next_seq(&[]), 1);
    }

    proptest! {
        #[test]
        fn an_undo_takes_the_pin_s_own_step_and_steps_in_order(seqs in proptest::collection::vec(1u64..50, 1..8)) {
            let steps: Vec<Step> = seqs
                .iter()
                .enumerate()
                .map(|(i, seq)| step(&format!("p{i}"), &[("f", None, *seq)]))
                .collect();
            let taken = undone_with(&steps, "p0");
            prop_assert!(taken.iter().any(|s| s.pin == "p0"));
            prop_assert!(taken.iter().all(|s| s.last() >= steps[0].last()));
            prop_assert!(taken.windows(2).all(|w| w[0].last() >= w[1].last()));
        }
    }
}
