//! The plant's Ports pane: what the product reaches outside itself, read off
//! its `ports.json` (contract Q), beside the tasks a person holds about a
//! port — one to approve, its secrets to provide — and the issues the
//! pane's buttons write. No I/O.

use std::fmt::Write as _;

use harness_core::domain::Issue;
use serde::{Deserialize, Serialize};

/// Where the ports are listed, at the product's root.
pub const PATH: &str = "ports.json";

/// What wraps a provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tech {
    /// `sdk`, `rest`, `graphql`, `sql`, `smtp`, `grpc` or `other`.
    pub kind: String,
    /// The SDK or the API.
    pub name: String,
    /// Its version, when pinned.
    pub version: String,
}

/// A port as `ports.json` lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Port {
    /// The business function: `payments`, `mail`, `database`.
    pub id: String,
    /// What the product uses it for.
    pub purpose: String,
    /// The provider keeps state: our data is the truth, only
    /// `DataCapabilities` write through it.
    pub stateful: bool,
    /// Who serves it now.
    pub provider: String,
    /// What wraps the provider.
    pub tech: Tech,
    /// The crate of the real adapter.
    pub adapter: String,
    /// The crate of the fake, empty for `database`.
    pub fake: String,
    /// The secrets' names — never their values.
    pub secrets: Vec<String>,
    /// What it costs, in words.
    pub cost: String,
    /// The Capabilities and `DataCapabilities` that go through it.
    pub used_by: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct File {
    ports: Vec<Port>,
}

/// The ports `ports.json` lists.
///
/// # Errors
///
/// Not the JSON of contract Q.
pub fn parse(text: &str) -> Result<Vec<Port>, String> {
    serde_json::from_str::<File>(text)
        .map(|f| f.ports)
        .map_err(|e| format!("ports.json does not read: {e}"))
}

/// A port a session asked for, waiting on a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Approval {
    /// The approval task.
    pub number: u64,
    /// The port asked.
    pub port: String,
    /// The task that waits on it.
    pub task: u64,
    /// What the session asked, as it said it: provider, cost, why.
    pub asked: String,
}

/// The title of the task that asks a person to approve `port` for `task`.
const APPROVE: &str = "Approve the port ";

/// The open approval task `issue` is, if it is one.
#[must_use]
pub fn approval(issue: &Issue) -> Option<Approval> {
    if !issue.state.eq_ignore_ascii_case("open") {
        return None;
    }
    let rest = issue.title.strip_prefix(APPROVE)?;
    let (port, rest) = rest.split_once(" for #")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let task = digits.parse().ok()?;
    let asked = issue
        .body
        .lines()
        .find_map(|l| l.strip_prefix("> "))
        .unwrap_or_default()
        .trim()
        .to_string();
    Some(Approval {
        number: issue.number,
        port: port.trim().to_string(),
        task,
        asked,
    })
}

/// A port's secrets, asked of a person at the factory's initialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Secrets {
    /// The task.
    pub number: u64,
    /// The port.
    pub port: String,
    /// The task is closed: the values are in `secrets/`, encrypted.
    pub provided: bool,
}

/// The secrets task `issue` is, if it is one.
#[must_use]
pub fn secrets(issue: &Issue) -> Option<Secrets> {
    let rest = issue.title.strip_prefix("Provide the secrets of the ")?;
    let (port, _) = rest.split_once(" port")?;
    Some(Secrets {
        number: issue.number,
        port: port.trim().to_string(),
        provided: issue.state.eq_ignore_ascii_case("closed"),
    })
}

/// What the pane shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Reading {
    /// The ports, as the integration branch lists them.
    pub ports: Vec<Port>,
    /// `ports.json` is not on the integration branch.
    pub missing: bool,
    /// The ports asked, waiting on a person.
    pub approvals: Vec<Approval>,
    /// The secrets asked, provided or not.
    pub secrets: Vec<Secrets>,
}

/// The pane's reading of `ports.json` — `None` when the branch has not —
/// and of the tasks a person holds.
///
/// # Errors
///
/// `ports.json` does not read.
pub fn reading(ports_json: Option<&str>, human: &[Issue]) -> Result<Reading, String> {
    let ports = ports_json.map(parse).transpose()?;
    Ok(Reading {
        missing: ports.is_none(),
        ports: ports.unwrap_or_default(),
        approvals: human.iter().filter_map(approval).collect(),
        secrets: human.iter().filter_map(secrets).collect(),
    })
}

/// What the approval task says when a person approves from the pane.
#[must_use]
pub fn approved(approval: &Approval) -> String {
    format!(
        "Approved from the plant's Ports pane: #{}'s next session adds the `{}` port to \
         ports.json and builds it, its fake and contract tests first.",
        approval.task, approval.port
    )
}

/// What the waiting task reads when a person refuses its port.
#[must_use]
pub fn refused(port: &str, why: &str) -> String {
    let why = why.trim();
    format!(
        "The port asked — `{port}` — is refused{}. Decide how this task does without it, \
         or cut it.",
        if why.is_empty() {
            String::new()
        } else {
            format!(": {why}")
        }
    )
}

/// A provider's name as a crate folder: `Amazon SES` → `amazon-ses`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// The roadmap item that moves `port` to `provider`: a new adapter behind
/// the same port, its contract tests green — and, for a port that keeps
/// state, our data replayed into the new provider first.
///
/// # Errors
///
/// No provider named, the one it has already, or the `database` port —
/// the product's own store, not abstracted.
pub fn change(port: &Port, provider: &str, why: &str) -> Result<(String, String), String> {
    let provider = provider.trim();
    if port.id == "database" {
        return Err(
            "the database is the product's own store: it has no adapter to change".to_string(),
        );
    }
    if slug(provider).is_empty() {
        return Err("name the new provider".to_string());
    }
    if slug(provider) == slug(&port.provider) {
        return Err(format!("{} already serves {}", port.provider, port.id));
    }
    let title = format!(
        "Change the provider of the {} port: {} → {provider}",
        port.id, port.provider
    );
    let mut body = format!(
        "The `{}` port ({}) moves from {} to {provider}.",
        port.id,
        port.purpose.trim(),
        port.provider
    );
    let why = why.trim();
    if !why.is_empty() {
        let _ = write!(body, " Why: {why}");
    }
    let _ = write!(
        body,
        "\n\n- A new adapter `crates/ports/{id}/{new}/` behind the same trait: the port's \
         contract tests pass on it, as on the fake.\n- ports.json: `{id}`'s provider, tech and \
         adapter name {provider}; the old adapter goes once nothing names it.\n- The secrets \
         {provider} needs are named in ports.json and asked of a person by a `harness:human` \
         task — never their values.\n- Used by {users}: none of them changes.",
        id = port.id,
        new = slug(provider),
        users = if port.used_by.is_empty() {
            "no unit yet".to_string()
        } else {
            port.used_by.join(", ")
        },
    );
    if port.stateful {
        let _ = write!(
            body,
            "\n- `{}` keeps state, and our data is the truth: replay it into {provider} \
             through a DataCapability before the switch.",
            port.id
        );
    }
    Ok((title, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const PORTS: &str = r#"{"version": 1, "ports": [
      {"id": "database", "purpose": "the store", "stateful": true, "provider": "PostgreSQL 16",
       "tech": {"kind": "sql", "name": "postgres", "version": "16"}, "adapter": "crates/data",
       "fake": "", "secrets": ["DATABASE_URL"], "usedBy": []},
      {"id": "payments", "purpose": "charge", "stateful": true, "provider": "Stripe",
       "tech": {"kind": "sdk", "name": "async-stripe"}, "adapter": "crates/ports/payments/stripe",
       "fake": "crates/ports/payments", "secrets": ["STRIPE_SECRET_KEY"], "cost": "2.9%",
       "usedBy": ["billing.charge"]}]}"#;

    fn issue(number: u64, title: &str, state: &str, body: &str) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            state: state.to_string(),
            body: body.to_string(),
            ..Issue::default()
        }
    }

    #[test]
    fn the_pane_reads_the_ports_and_the_tasks_a_person_holds() {
        let human = [
            issue(
                21,
                "Approve the port mail for #11: Send receipts",
                "open",
                "needs:\n\n> mail — Postmark, $15 a month, receipts (/code)\n\nmore",
            ),
            issue(22, "Approve the port llm for #12: Summaries", "closed", ""),
            issue(
                23,
                "Provide the secrets of the payments port (Stripe)",
                "closed",
                "",
            ),
            issue(
                24,
                "Provide the secrets of the mail port (Postmark)",
                "open",
                "",
            ),
            issue(25, "Decide for #13: Pay", "open", ""),
        ];
        let read = reading(Some(PORTS), &human).expect("reads");
        assert!(!read.missing);
        assert_eq!(read.ports.len(), 2);
        assert_eq!(read.ports[1].used_by, vec!["billing.charge".to_string()]);
        assert_eq!(read.ports[1].tech.name, "async-stripe");
        assert_eq!(
            read.approvals,
            vec![Approval {
                number: 21,
                port: "mail".to_string(),
                task: 11,
                asked: "mail — Postmark, $15 a month, receipts (/code)".to_string(),
            }],
            "a closed approval waits on no one"
        );
        assert_eq!(
            read.secrets
                .iter()
                .map(|s| (s.port.as_str(), s.provided))
                .collect::<Vec<_>>(),
            vec![("payments", true), ("mail", false)]
        );
        let none = reading(None, &[]).expect("reads");
        assert!(none.missing && none.ports.is_empty());
        assert!(reading(Some("{"), &[]).is_err());
    }

    #[test]
    fn approving_and_refusing_say_what_comes_next() {
        let asked = Approval {
            number: 21,
            port: "mail".to_string(),
            task: 11,
            asked: String::new(),
        };
        assert!(approved(&asked).contains("#11's next session adds the `mail` port"));
        assert_eq!(
            refused("mail", " too dear "),
            "The port asked — `mail` — is refused: too dear. Decide how this task does without it, or cut it."
        );
        assert!(refused("mail", "").starts_with("The port asked — `mail` — is refused. "));
    }

    #[test]
    fn a_change_of_provider_is_a_new_adapter_and_a_replay_when_it_keeps_state() {
        let ports = parse(PORTS).expect("reads");
        let (title, body) = change(&ports[1], " Adyen Payments ", "fees").expect("a change");
        assert_eq!(
            title,
            "Change the provider of the payments port: Stripe → Adyen Payments"
        );
        assert!(
            body.contains("crates/ports/payments/adyen-payments/"),
            "{body}"
        );
        assert!(body.contains("Why: fees"), "{body}");
        assert!(body.contains("Used by billing.charge"), "{body}");
        assert!(body.contains("replay it into Adyen Payments"), "{body}");
        let mut stateless = ports[1].clone();
        stateless.stateful = false;
        stateless.used_by.clear();
        let (_, body) = change(&stateless, "Adyen", "").expect("a change");
        assert!(!body.contains("replay") && !body.contains("Why:"), "{body}");
        assert!(body.contains("Used by no unit yet"), "{body}");
        assert_eq!(
            change(&ports[1], " ", ""),
            Err("name the new provider".to_string())
        );
        assert_eq!(
            change(&ports[1], "stripe", ""),
            Err("Stripe already serves payments".to_string())
        );
        assert!(change(&ports[0], "MySQL", "").is_err());
    }

    proptest! {
        #[test]
        fn reading_any_title_or_file_never_panics(title in ".{0,80}", text in ".{0,200}") {
            let _ = approval(&issue(1, &title, "open", &text));
            let _ = secrets(&issue(1, &title, "closed", ""));
            let _ = parse(&text);
        }
    }
}
