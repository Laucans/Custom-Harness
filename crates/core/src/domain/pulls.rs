//! What you read from a PR. Pure domain: no I/O, no subprocesses.

/// A PR, as a review reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pr {
    /// Its number, as text — it only appears in prose and paths.
    pub num: String,
    /// The targeted branch.
    pub base: String,
    /// The branch that carries the change.
    pub head: String,
    /// Its title.
    pub title: String,
    /// Its URL.
    pub url: String,
    /// `OPEN`, `CLOSED`, `MERGED` — as the API renders it, uninterpreted.
    pub state: String,
    /// A draft has nothing to be reviewed.
    pub draft: bool,
}

impl Pr {
    /// `#12` — as a message names it.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("#{}", self.num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_is_the_number_prefixed() {
        let pr = Pr {
            num: "32".to_string(),
            ..Pr::default()
        };
        assert_eq!(pr.reference(), "#32");
    }
}
