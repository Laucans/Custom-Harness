//! What this run changes for a re-study, by value.

use std::path::PathBuf;
use std::rc::Rc;

use harness_core::domain::prompt_book::PromptBook;
use harness_core::ports::agent::SessionSpec;

/// The settings of a re-study.
pub struct Config {
    /// The tasks' lock directory — one re-study per task at a time.
    pub restudy_dir: PathBuf,
    /// Model and effort for the session that studies.
    pub restudy: SessionSpec,
    /// The human's prompt edits, read once per run (see [`crate::prompts`]).
    pub prompts: Rc<PromptBook>,
}

/// The ports of the product's rules runtime a re-study of `task` serves on.
///
/// The server's, then the app's. Every code session serves on the
/// runtime's defaults (8790, 8791): a re-study beside one on another lane
/// takes a pair of its own, `20000 + (task % 5000) * 2`.
#[must_use]
pub const fn ports_pair(task: u64) -> (u16, u16) {
    // `task % 5000` is under 5000: the pair stays under 30000.
    #[allow(clippy::cast_possible_truncation)]
    let server = 20_000 + (task % 5_000) as u16 * 2;
    (server, server + 1)
}

#[cfg(test)]
pub(crate) mod fake {
    //! A test config, disposable folder.

    use std::path::PathBuf;

    use harness_core::ports::agent::SessionSpec;

    use super::Config;

    /// A test config, under this folder.
    pub fn in_dir(restudy_dir: PathBuf) -> Config {
        Config {
            prompts: std::rc::Rc::default(),
            restudy_dir,
            restudy: SessionSpec {
                model: "opus".to_string(),
                effort: "high".to_string(),
            },
        }
    }

    /// A test config, disposable folder.
    pub fn config() -> Config {
        in_dir(PathBuf::from("/tmp/restudy"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_re_study_serves_on_its_own_pair_of_ports() {
        assert_eq!(ports_pair(219), (20_438, 20_439));
        assert_eq!(ports_pair(0), (20_000, 20_001));
        assert_eq!(ports_pair(4_999), (29_998, 29_999));
        assert_eq!(ports_pair(5_219), ports_pair(219));
        assert_ne!(ports_pair(219).0, 8_790);
    }
}
