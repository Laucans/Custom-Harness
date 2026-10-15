//! A creation's ports step: every dependency the product needs outside
//! itself, as the AI reads them off the brief, the decisions, the rules, the
//! data model and the micro-frontends, then chosen by the human — the
//! provider of each, what wraps it, whether it keeps state — before the
//! factory is initialized and writes them as `ports.json` (contract Q).
//!
//! PostgreSQL is always there, the `database` port: listed, not abstracted.
//! Pure: the asking is the desk's.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::creation::{Context, Creation, Family};

/// The kinds of technology a port wraps, as contract Q lists them.
pub const TECH_KINDS: [&str; 7] = ["sdk", "rest", "graphql", "sql", "smtp", "grpc", "other"];

/// What the ports step drafted and the human settled.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortsDraft {
    /// The ports, `database` first.
    pub ports: Vec<PortDraft>,
    /// The rules and the data model it was drafted from: what says it is
    /// stale.
    pub from: String,
    /// The human validated it.
    pub validated: bool,
}

/// One port as the step holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortDraft {
    /// The business function: `payments`, never `stripe`.
    pub id: String,
    /// What the product needs from outside, in a sentence.
    pub purpose: String,
    /// The provider keeps state: our Data is the truth, only
    /// `DataCapabilities` write through it.
    pub stateful: bool,
    /// The provider chosen.
    pub provider: String,
    /// What wraps it: `sdk`, `rest`, …
    pub tech_kind: String,
    /// The SDK, crate, package or API.
    pub tech_name: String,
    /// Its version, when it matters.
    pub tech_version: String,
    /// The secrets it needs, by name.
    pub secrets: Vec<String>,
    /// How it is paid, its quotas.
    pub cost: String,
    /// The Capabilities and `DataCapabilities` that go through it.
    pub used_by: Vec<String>,
    /// Other providers the AI weighed, for the human to pick from.
    pub alternatives: Vec<String>,
}

/// The `database` port every product has: its own PostgreSQL.
#[must_use]
pub fn database() -> PortDraft {
    PortDraft {
        id: "database".to_string(),
        purpose: "The product's store: the Data layer reads it, the DataGuard writes it — listed, not abstracted.".to_string(),
        stateful: true,
        provider: "PostgreSQL 16".to_string(),
        tech_kind: "sql".to_string(),
        tech_name: "postgres".to_string(),
        tech_version: "16".to_string(),
        secrets: vec!["DATABASE_URL".to_string()],
        cost: "the host's".to_string(),
        used_by: Vec::new(),
        alternatives: Vec::new(),
    }
}

/// Whether a name is a port's id: `kebab-case`, starting with a letter.
fn is_id(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Whether a name is a secret's: `UPPER_SNAKE`.
fn is_secret(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// A provider's name as a crate folder: `Google Cloud` → `google-cloud`.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// What the ports are drafted from, as one print: a change of the rules or
/// of the data model's writes makes them stale.
#[must_use]
pub fn ports_print(c: &Creation) -> String {
    let mut print = String::new();
    for r in &c.requirements {
        let _ = write!(print, "R{}:{}|", r.id, r.title.trim());
    }
    if let Some(d) = &c.data {
        for (write, fields) in &d.writes {
            let _ = write!(print, "{write}={}|", fields.join(","));
        }
    }
    print
}

/// The question the ports are drafted with.
#[must_use]
pub fn ports_prompt(c: &Creation, ctx: &Context) -> String {
    let mut text = String::from(
        "You list the dependencies a product needs OUTSIDE itself, each behind a port named by \
         its business function — `payments`, `mail`, `object-storage`, `llm`, `identity`, \
         `geocoding`, `search`… never the provider's name — and propose a provider for each. \
         A port is anything the product reaches beyond its own code and its own PostgreSQL \
         (the `database` port is already there: do not list it): a third-party API, a SaaS, a \
         payment provider, mail, object storage, an LLM, an identity provider. A library is not \
         a port. List only what the brief, the decisions, the rules and the screens below \
         actually need — none at all is a valid answer.\n\n\
         For each port: `stateful` is true when the provider keeps state the product relies on \
         (a customer, a subscription, a file, a user) — then only DataCapabilities go through \
         it and our data stays the truth; false for a stateless service (an LLM, a geocoder). \
         `usedBy` names the Capabilities (reads, stateless ports only) and DataCapabilities \
         (writes) below that need it, by their identifiers. `tech_kind` is one of sdk, rest, \
         graphql, sql, smtp, grpc, other; `tech_name` the SDK, crate or API; `secrets` the \
         environment names it needs (UPPER_SNAKE, never values); `cost` how it is paid; \
         `alternatives` two or three other providers that would do.\n\n\
         Answer with ONLY a JSON array:\n\
         [{\"id\": \"payments\", \"purpose\": \"charge the subscription\", \"stateful\": true, \
         \"provider\": \"Stripe\", \"tech_kind\": \"sdk\", \"tech_name\": \"async-stripe\", \
         \"tech_version\": \"0.39\", \"secrets\": [\"STRIPE_SECRET_KEY\"], \"cost\": \"1.5% + \
         0.25€ per charge\", \"used_by\": [\"billing.chargeSubscription\"], \"alternatives\": \
         [\"Mollie\", \"Adyen\"]}]\n\n",
    );
    let _ = write!(
        text,
        "## What the application does\n\n{}\n\n",
        c.brief.trim()
    );
    let answered: Vec<String> = c
        .questions
        .iter()
        .filter(|q| q.answered())
        .map(|q| format!("- {} — {}", q.title, q.answer.trim()))
        .collect();
    if !answered.is_empty() {
        let _ = write!(text, "## Decisions\n\n{}\n\n", answered.join("\n"));
    }
    let _ = write!(text, "## Scope\n\n{}\n\n## Rules\n\n", c.scope.trim());
    for r in &c.requirements {
        let family = if r.family == Family::Business {
            "business"
        } else {
            "journey"
        };
        let _ = writeln!(
            text,
            "- R{} ({family}) {} — {}",
            r.id,
            r.title.trim(),
            r.body.trim()
        );
    }
    text.push_str("\n## Micro-frontends, with what they read and trigger\n\n");
    for m in &ctx.components {
        let _ = writeln!(
            text,
            "- {} ({}): reads {}; triggers {}",
            m.name,
            m.system,
            m.needs.join(", "),
            m.actions.join(", ")
        );
    }
    if let Some(d) = &c.data {
        let tables: Vec<&str> = d.tables.iter().map(|t| t.name.as_str()).collect();
        let _ = write!(text, "\n## Data model\n\nTables: {}\n", tables.join(", "));
    }
    text
}

/// The ports an answer carries, `database` first, each id once.
///
/// # Errors
///
/// No JSON array of ports.
pub fn parse_ports(answer: &str, from: &str) -> Result<PortsDraft, String> {
    let start = answer.find('[').ok_or("the answer carries no JSON array")?;
    let end = answer
        .rfind(']')
        .ok_or("the answer carries no JSON array")?;
    let text = answer
        .get(start..=end)
        .ok_or("the answer carries no JSON array")?;
    let read: Vec<PortDraft> =
        serde_json::from_str(text).map_err(|e| format!("the ports are not readable: {e}"))?;
    let mut ports = vec![database()];
    let mut seen: BTreeSet<String> = BTreeSet::from(["database".to_string()]);
    for mut port in read {
        port.id = slug(&port.id);
        if port.id.is_empty() || !seen.insert(port.id.clone()) {
            continue;
        }
        port.secrets.retain(|s| is_secret(s));
        ports.push(port);
    }
    Ok(PortsDraft {
        ports,
        from: from.to_string(),
        validated: false,
    })
}

/// The human's ports, as the page sends them back whole.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PortsEdit {
    /// Every port, in order.
    pub ports: Vec<PortDraft>,
}

impl PortsDraft {
    /// The human's list applied: `database` kept first whatever they sent,
    /// the validation taken back.
    pub fn edit(&mut self, edit: PortsEdit) {
        let mut ports = vec![
            self.ports
                .iter()
                .find(|p| p.id == "database")
                .cloned()
                .unwrap_or_else(database),
        ];
        ports.extend(edit.ports.into_iter().filter(|p| p.id != "database"));
        if ports != self.ports {
            self.ports = ports;
            self.validated = false;
        }
    }

    /// What keeps it from validation, in words; empty when nothing. `reads`
    /// and `writes` are the Capabilities and `DataCapabilities` the mock-up
    /// names.
    #[must_use]
    pub fn gaps(&self, reads: &BTreeSet<String>, writes: &BTreeSet<String>) -> Vec<String> {
        let mut gaps = Vec::new();
        let mut seen = BTreeSet::new();
        for p in &self.ports {
            let name = if p.id.is_empty() {
                "a port"
            } else {
                p.id.as_str()
            };
            if !is_id(&p.id) {
                gaps.push(format!(
                    "{name}: its id is a business function in kebab-case"
                ));
            } else if !seen.insert(p.id.as_str()) {
                gaps.push(format!("{name} is there twice"));
            }
            if p.provider.trim().is_empty() {
                gaps.push(format!("{name} has no provider"));
            }
            if !TECH_KINDS.contains(&p.tech_kind.as_str()) || p.tech_name.trim().is_empty() {
                gaps.push(format!("{name} does not say what wraps its provider"));
            }
            if let Some(bad) = p.secrets.iter().find(|s| !is_secret(s)) {
                gaps.push(format!(
                    "{name}: the secret `{bad}` is not an UPPER_SNAKE name"
                ));
            }
            for unit in &p.used_by {
                if p.stateful && reads.contains(unit) {
                    gaps.push(format!(
                        "{name} keeps state: {unit} reads our mirror, not the provider"
                    ));
                } else if !reads.contains(unit) && !writes.contains(unit) {
                    gaps.push(format!("{name}: {unit} is no unit of the mock-up"));
                }
            }
        }
        gaps
    }

    /// The ports as contract Q has them: `ports.json`.
    ///
    /// # Errors
    ///
    /// Never in practice: plain data.
    pub fn ports_json(&self) -> Result<String, String> {
        let ports: Vec<serde_json::Value> = self
            .ports
            .iter()
            .map(|p| {
                let database = p.id == "database";
                let mut tech = serde_json::json!({ "kind": p.tech_kind, "name": p.tech_name });
                if !p.tech_version.trim().is_empty() {
                    tech["version"] = serde_json::json!(p.tech_version.trim());
                }
                let mut port = serde_json::json!({
                    "id": p.id,
                    "purpose": p.purpose.trim(),
                    "stateful": p.stateful,
                    "provider": p.provider.trim(),
                    "tech": tech,
                    "adapter": if database {
                        "crates/data".to_string()
                    } else {
                        format!("crates/ports/{}/{}", p.id, slug(&p.provider))
                    },
                    "fake": if database { String::new() } else { format!("crates/ports/{}", p.id) },
                    "secrets": p.secrets,
                    "usedBy": p.used_by,
                });
                if !p.cost.trim().is_empty() {
                    port["cost"] = serde_json::json!(p.cost.trim());
                }
                port
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({ "version": 1, "ports": ports }))
            .map(|text| text + "\n")
            .map_err(|e| e.to_string())
    }
}

/// The body of the task that asks a human for a port's secrets.
#[must_use]
pub fn secrets_body(port: &PortDraft) -> String {
    format!(
        "The `{id}` port ({provider}) needs these secrets: {names}.\n\n\
         Put their values in `secrets/<env>.enc.yaml`, encrypted with sops and the age key of \
         `.sops.yaml` (`sops secrets/production.enc.yaml`), commit, then close this issue. Never \
         commit a value in clear — the gate `secrets-encrypted` refuses it. Nothing in \
         development waits on them: the tests run on the port's fake.",
        id = port.id,
        provider = port.provider,
        names = port
            .secrets
            .iter()
            .map(|s| format!("`{s}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn units() -> (BTreeSet<String>, BTreeSet<String>) {
        (
            BTreeSet::from([
                "billing.listInvoices".to_string(),
                "search.find".to_string(),
            ]),
            BTreeSet::from(["billing.chargeSubscription".to_string()]),
        )
    }

    fn answer() -> &'static str {
        r#"Here: [{"id": "Payments", "purpose": "charge", "stateful": true, "provider": "Stripe",
            "tech_kind": "sdk", "tech_name": "async-stripe", "secrets": ["STRIPE_SECRET_KEY", "bad key"],
            "cost": "1.5%", "used_by": ["billing.chargeSubscription"], "alternatives": ["Mollie"]},
           {"id": "database", "provider": "MySQL"},
           {"id": "payments", "provider": "twice"}]"#
    }

    #[test]
    fn the_ports_read_back_the_database_first_each_once() {
        let draft = parse_ports(answer(), "print").expect("read");
        let ids: Vec<&str> = draft.ports.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["database", "payments"]);
        assert_eq!(
            draft.ports[0].provider, "PostgreSQL 16",
            "ours, never the AI's"
        );
        assert_eq!(draft.ports[1].secrets, ["STRIPE_SECRET_KEY"]);
        assert_eq!(draft.from, "print");
        assert!(parse_ports("none", "").is_err());
        assert!(
            parse_ports("[]", "").expect("none").ports.len() == 1,
            "no port is an answer"
        );
    }

    #[test]
    fn a_draft_is_validated_once_every_port_is_whole() {
        let (reads, writes) = units();
        let draft = parse_ports(answer(), "").expect("read");
        assert!(
            draft.gaps(&reads, &writes).is_empty(),
            "{:?}",
            draft.gaps(&reads, &writes)
        );
        let mut bad = draft;
        bad.ports[1]
            .used_by
            .push("billing.listInvoices".to_string());
        bad.ports[1].provider.clear();
        bad.ports.push(PortDraft {
            id: "Mail".to_string(),
            tech_kind: "carrier pigeon".to_string(),
            used_by: vec!["nowhere.sendMail".to_string()],
            ..PortDraft::default()
        });
        let gaps = bad.gaps(&reads, &writes);
        for said in [
            "payments has no provider",
            "payments keeps state: billing.listInvoices reads our mirror",
            "Mail: its id is a business function",
            "Mail does not say what wraps",
            "Mail: nowhere.sendMail is no unit",
        ] {
            assert!(gaps.iter().any(|g| g.contains(said)), "{said}: {gaps:?}");
        }
    }

    #[test]
    fn an_edit_keeps_the_database_and_takes_the_validation_back() {
        let mut draft = parse_ports(answer(), "").expect("read");
        draft.validated = true;
        draft.edit(PortsEdit {
            ports: vec![PortDraft {
                id: "database".to_string(),
                provider: "MySQL".to_string(),
                ..PortDraft::default()
            }],
        });
        assert_eq!(draft.ports.len(), 1);
        assert_eq!(draft.ports[0].provider, "PostgreSQL 16");
        assert!(!draft.validated);
        let same = draft.clone();
        draft.validated = true;
        draft.edit(PortsEdit { ports: Vec::new() });
        assert!(draft.validated, "nothing changed: still validated");
        assert_eq!(draft.ports, same.ports);
    }

    #[test]
    fn the_ports_are_written_as_contract_q_has_them() {
        let draft = parse_ports(answer(), "").expect("read");
        let json: serde_json::Value =
            serde_json::from_str(&draft.ports_json().expect("json")).expect("json");
        assert_eq!(json["version"], 1);
        assert_eq!(json["ports"][0]["adapter"], "crates/data");
        assert_eq!(json["ports"][0]["fake"], "");
        assert_eq!(json["ports"][1]["adapter"], "crates/ports/payments/stripe");
        assert_eq!(json["ports"][1]["fake"], "crates/ports/payments");
        assert_eq!(json["ports"][1]["tech"]["name"], "async-stripe");
        assert_eq!(json["ports"][1]["usedBy"][0], "billing.chargeSubscription");
        assert!(
            json["ports"][1].get("alternatives").is_none(),
            "the AI's options stay here"
        );
        assert!(
            json["ports"][1]
                .get("tech")
                .and_then(|t| t.get("version"))
                .is_none()
        );
    }

    proptest! {
        #[test]
        fn reading_any_answer_never_panics(text in ".{0,300}") {
            let _ = parse_ports(&text, "");
        }

        #[test]
        fn a_slug_is_always_a_port_id_or_empty(name in "[A-Za-z0-9 ._-]{0,30}") {
            let s = slug(&name);
            prop_assert!(s.is_empty() || is_id(&s) || s.starts_with(|c: char| c.is_ascii_digit()));
        }
    }
}
