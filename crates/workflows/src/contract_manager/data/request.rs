//! The request as the dev loop wrote it on the `harness:contract-change`
//! issue, read back here with no I/O.

/// What a consumer asked: the contract, who asked, what it needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    /// The id whose contract must change.
    pub id: String,
    /// The task that stopped on it.
    pub task: u64,
    /// That task's milestone, when the issue names one.
    pub milestone: Option<u64>,
    /// What the consumer needs of the contract, in its words.
    pub need: String,
}

impl Request {
    /// The body the dev loop writes: one `key: value` line each, then the
    /// prose. The title names the id and the task for a reader.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "id: {}\ntask: #{}\nmilestone: {}\nneed: {}\n\nThe session building #{} needs the \
             contract of `{}` to change — a contract another milestone produces, which it may not \
             edit (gate `contracts-untouched`). The contract manager evolves it: a new version of \
             its manifest and of the fixture that stands for it, a task on the producer's \
             milestone, a note to every consumer. #{} waits on this issue.",
            self.id,
            self.task,
            self.milestone
                .map_or_else(|| "-".to_string(), |m| format!("#{m}")),
            self.need,
            self.task,
            self.id,
            self.task
        )
    }

    /// The request a body holds; `None` without an `id:` and a `task:`.
    #[must_use]
    pub fn parse(body: &str) -> Option<Self> {
        let value = |key: &str| -> Option<String> {
            body.lines()
                .map(str::trim)
                .find_map(|line| line.strip_prefix(key))
                .map(|rest| rest.trim().to_string())
                .filter(|rest| !rest.is_empty())
        };
        let number = |text: &str| text.trim_start_matches('#').parse::<u64>().ok();
        let id = value("id:")?.trim_matches('`').to_string();
        let task = number(&value("task:")?)?;
        Some(Self {
            id,
            task,
            milestone: value("milestone:").and_then(|m| number(&m)),
            need: value("need:").unwrap_or_default(),
        })
    }

    /// The issue's title.
    #[must_use]
    pub fn title(&self) -> String {
        format!("Contract change: {} for #{}", self.id, self.task)
    }
}

/// The stop a consumer makes: `AGENT_LOOP_STOP: contract <id> — <need>`,
/// read as the id and the need. `None` when the reason asks nothing of a
/// contract.
#[must_use]
pub fn asked(reason: &str) -> Option<(String, String)> {
    let (_, after) = reason.split_once("AGENT_LOOP_STOP")?;
    let asked = after
        .trim_start_matches(':')
        .trim_start()
        .strip_prefix("contract ")?
        .trim();
    let (id, need) = asked
        .split_once(" — ")
        .or_else(|| asked.split_once(" - "))
        .or_else(|| asked.split_once(": "))
        .map_or((asked, ""), |(id, need)| (id, need));
    let id = id.trim().trim_matches('`').trim_end_matches(':');
    (!id.is_empty() && !id.contains(char::is_whitespace))
        .then(|| (id.to_string(), need.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_names_the_contract_and_what_it_needs() {
        assert_eq!(
            asked("AGENT_LOOP_STOP: contract session.listScenes — the scenes' media ids"),
            Some((
                "session.listScenes".to_string(),
                "the scenes' media ids".to_string()
            ))
        );
        assert_eq!(
            asked("stage code: AGENT_LOOP_STOP: contract `media.upload` - a mime type (/code)"),
            Some((
                "media.upload".to_string(),
                "a mime type (/code)".to_string()
            ))
        );
        assert_eq!(
            asked("AGENT_LOOP_STOP: contract x.y"),
            Some(("x.y".to_string(), String::new()))
        );
        assert_eq!(asked("AGENT_LOOP_STOP: port payments — Stripe"), None);
        assert_eq!(asked("AGENT_LOOP_STOP: contract "), None);
        assert_eq!(asked("AGENT_LOOP_STOP: contract two words"), None);
        assert_eq!(asked("all green"), None);
    }

    #[test]
    fn the_request_reads_back_what_it_wrote() {
        let request = Request {
            id: "session.listScenes".to_string(),
            task: 61,
            milestone: Some(19),
            need: "the scenes' media ids".to_string(),
        };
        let body = request.render();
        assert_eq!(Request::parse(&body), Some(request.clone()));
        assert!(body.contains("contracts-untouched") && body.contains("#61 waits"));
        assert_eq!(
            request.title(),
            "Contract change: session.listScenes for #61"
        );
        let no_milestone = Request {
            milestone: None,
            ..request
        };
        assert_eq!(
            Request::parse(&no_milestone.render()),
            Some(no_milestone.clone())
        );
        assert!(no_milestone.render().contains("milestone: -"));
    }

    #[test]
    fn a_body_without_an_id_or_a_task_is_no_request() {
        assert_eq!(Request::parse("task: #3\nneed: x"), None);
        assert_eq!(Request::parse("id: a.b\nneed: x"), None);
        assert_eq!(Request::parse("id: a.b\ntask: nope"), None);
        assert_eq!(
            Request::parse("id: `a.b`\ntask: 3"),
            Some(Request {
                id: "a.b".to_string(),
                task: 3,
                milestone: None,
                need: String::new(),
            })
        );
    }
}
