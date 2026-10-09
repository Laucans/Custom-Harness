//! The diagnosis as the session answers it: a JSON object, read back
//! tolerant of prose and of a fenced block around it.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// What the session concluded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Diagnosis {
    /// The state of affairs, in two or three sentences.
    pub summary: String,
    /// One per divergence.
    pub findings: Vec<Finding>,
    /// What a human can do, an imperative sentence each.
    pub actions: Vec<String>,
}

/// One divergence, judged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Finding {
    /// The divergence, as the comparison listed it.
    pub divergence: String,
    /// `database`, `model`, `both` or `unclear`.
    pub wrong: String,
    /// Why, from the history.
    pub why: String,
    /// The change to make.
    pub fix: String,
}

impl Diagnosis {
    /// Reads the session's reply.
    ///
    /// # Errors
    /// The sentence a journal line says: no JSON object in the reply, or one
    /// that is not a diagnosis.
    pub fn parse(text: &str) -> Result<Self, String> {
        let candidate = extract(text).ok_or_else(|| "no JSON object in the reply".to_string())?;
        serde_json::from_str(&candidate).map_err(|e| format!("the JSON did not parse: {e}"))
    }

    /// The diagnosis as a human reads it: the summary, then each finding.
    #[must_use]
    pub fn markdown(&self) -> String {
        let mut text = self.summary.trim().to_string();
        for f in &self.findings {
            let _ = write!(
                text,
                "\n\n**{}** — wrong: **{}**\n{}\n→ {}",
                f.divergence.trim(),
                if f.wrong.trim().is_empty() {
                    "unclear"
                } else {
                    f.wrong.trim()
                },
                f.why.trim(),
                f.fix.trim()
            );
        }
        text
    }
}

/// A fenced ` ```json ` block when there is one, else the span from the
/// first `{` to the last `}`.
fn extract(text: &str) -> Option<String> {
    if let Some(start) = text.find("```json") {
        let after = text.get(start + 7..)?;
        if let Some(end) = after.find("```") {
            return after.get(..end).map(|s| s.trim().to_string());
        }
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start)
        .then(|| text.get(start..=end).map(str::to_string))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const REPLY: &str = "Here is what I found.\n```json\n{\"summary\": \"The model is behind.\", \
        \"findings\": [{\"divergence\": \"field pj.origine is in the database, not in the model\", \
        \"wrong\": \"model\", \"why\": \"0007 added it; model.json untouched.\", \
        \"fix\": \"Add origine to pj in data/model.json.\"}], \
        \"actions\": [\"Describe pj.origine in data/model.json.\"]}\n```\nDone.";

    #[test]
    fn a_fenced_reply_reads_back() {
        let d = Diagnosis::parse(REPLY).expect("a diagnosis");
        assert_eq!(d.summary, "The model is behind.");
        assert_eq!(d.findings[0].wrong, "model");
        assert_eq!(d.actions.len(), 1);
        let md = d.markdown();
        assert!(md.starts_with("The model is behind."));
        assert!(md.contains("wrong: **model**"));
    }

    #[test]
    fn a_bare_object_reads_and_prose_alone_does_not() {
        assert!(Diagnosis::parse("{\"summary\": \"x\"}").is_ok());
        assert!(Diagnosis::parse("nothing to say").is_err());
        assert!(Diagnosis::parse("} backwards {").is_err());
    }

    proptest! {
        #[test]
        fn any_reply_reads_without_a_panic(text in "\\PC{0,300}") {
            let _ = Diagnosis::parse(&text);
        }
    }
}
