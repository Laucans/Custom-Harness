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

/// The task the contract manager opened to build a change, and the
/// milestone it went under — what a PR waiting on the change waits on in
/// truth (`common::contest::contract_landed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Producer {
    /// The producer task (`#582`).
    pub task: u64,
    /// The milestone it was opened under, when it went under one.
    pub milestone: Option<u64>,
}

/// What opens the line the contract manager writes on a request it
/// answered with a producer task: `producer: #582 on #30`.
const PRODUCER: &str = "producer:";

/// The request's header: its lines before the first blank one — the
/// `key: value` lines [`Request::render`] writes, before the prose.
fn header(body: &str) -> impl Iterator<Item = &str> {
    body.lines().take_while(|line| !line.trim().is_empty())
}

/// `body` with its producer line at the end of the request's header.
///
/// The line is `producer: #<task> on #<milestone>` (or `on -`); the header
/// is the request's `key: value` lines ([`header`]). One already in the
/// header is replaced, and a line of the prose that happens to start with
/// `producer:` is left as it is.
#[must_use]
pub fn with_producer(body: &str, producer: Producer) -> String {
    let line = format!(
        "{PRODUCER} #{} on {}",
        producer.task,
        producer
            .milestone
            .map_or_else(|| "-".to_string(), |m| format!("#{m}"))
    );
    let kept = header(body).count();
    let mut lines: Vec<String> = header(body)
        .filter(|kept| !kept.trim_start().starts_with(PRODUCER))
        .map(ToString::to_string)
        .collect();
    lines.push(line);
    lines.extend(body.lines().skip(kept).map(ToString::to_string));
    lines.join("\n")
}

/// The producer task a request records in its header ([`with_producer`]);
/// `None` for a request answered without one — handed back, the
/// requester's own, or written before the line existed.
#[must_use]
pub fn producer(body: &str) -> Option<Producer> {
    let rest = header(body)
        .map(str::trim)
        .find_map(|line| line.strip_prefix(PRODUCER))?
        .trim();
    let (task, milestone) = rest.split_once(" on ").unwrap_or((rest, "-"));
    let number = |text: &str| text.trim().trim_start_matches('#').parse::<u64>().ok();
    Some(Producer {
        task: number(task)?,
        milestone: number(milestone),
    })
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
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn a_producer_line_reads_back_leaves_the_request_whole_and_writes_once(
            task in 1u64..1_000_000,
            milestone in proptest::option::of(1u64..100_000),
            asker in 1u64..1_000_000,
            need in "[a-zA-Z0-9#:.,'()-]([a-zA-Z0-9#:.,'() -]{0,40}[a-zA-Z0-9#:.,'()-])?",
            crlf in any::<bool>(),
            prose in "(producer: #9 on #9 later in the prose)?",
        ) {
            let request = Request {
                id: "campaign.listCampaigns".to_string(),
                task: asker,
                milestone,
                need,
            };
            let mut body = format!("{}\n{prose}", request.render());
            if crlf {
                body = body.replace('\n', "\r\n");
            }
            let built = Producer { task, milestone };
            let written = with_producer(&body, built);
            prop_assert_eq!(producer(&written), Some(built));
            prop_assert_eq!(Request::parse(&written), Request::parse(&body));
            prop_assert_eq!(&with_producer(&written, built), &written);
            // The prose is left as it was.
            prop_assert_eq!(written.contains(&prose), true);
        }
    }

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
    fn the_producer_line_reads_back_and_leaves_the_request_whole() {
        // ForgeDesMondes #581: answered by producer #582 under milestone #30.
        let request = Request {
            id: "campaign.listCampaigns".to_string(),
            task: 576,
            milestone: Some(30),
            need: "sessions counted from the session table".to_string(),
        };
        let body = request.render();
        assert_eq!(producer(&body), None, "a request answered by nobody yet");
        let built = Producer {
            task: 582,
            milestone: Some(30),
        };
        let written = with_producer(&body, built);
        assert_eq!(producer(&written), Some(built));
        assert_eq!(Request::parse(&written), Some(request));
        assert!(
            written.contains(
                "need: sessions counted from the session table\nproducer: #582 on #30\n\n"
            ),
            "{written}"
        );
        // Written again, it replaces the line; a producer under no milestone.
        let moved = with_producer(
            &written,
            Producer {
                task: 590,
                milestone: None,
            },
        );
        assert_eq!(moved.matches("producer:").count(), 1, "{moved}");
        assert_eq!(
            producer(&moved),
            Some(Producer {
                task: 590,
                milestone: None
            })
        );
        // A body with no blank line takes it at its end; a line with no task
        // number is none.
        assert_eq!(
            with_producer("id: a.b", built),
            "id: a.b\nproducer: #582 on #30"
        );
        assert_eq!(producer("producer: someone"), None);
        // Only the header says it: a prose line is neither read nor dropped.
        assert_eq!(producer("id: a.b\n\nproducer: #5 on #6"), None);
        assert_eq!(
            with_producer("id: a.b\n\nproducer: #5 on #6", built),
            "id: a.b\nproducer: #582 on #30\n\nproducer: #5 on #6"
        );
        assert_eq!(
            producer("producer: #7"),
            Some(Producer {
                task: 7,
                milestone: None
            })
        );
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
