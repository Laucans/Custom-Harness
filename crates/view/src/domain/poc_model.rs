//! The proof of concept's foundations, drafted as soon as its scope is
//! settled: the data it keeps (contract M's shape) and the Capabilities and
//! `DataCapabilities` its journeys need — then set beside what the mock-up's
//! micro-frontends say they read and trigger, so the review sees the gaps.
//!
//! Drafts only: they become tasks once the proof of concept is accepted.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::creation::{ComponentRef, Creation, product};

/// What a port does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortKind {
    /// A Capability: it reads.
    #[default]
    Capability,
    /// A `DataCapability`: it writes.
    DataCapability,
}

/// One Capability or `DataCapability` the journeys need.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Port {
    /// `<system>.<verbThing>`.
    pub id: String,
    /// Whether it reads or writes.
    pub kind: PortKind,
    /// What it is for, in a sentence.
    pub purpose: String,
}

/// The drafts, as the record keeps them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Drafts {
    /// The data model, contract M's shape: `{"version": 1, "tables": […]}`.
    pub model: Value,
    /// The ports, in the order the journeys need them.
    pub ports: Vec<Port>,
}

impl Drafts {
    /// The tables the model drafts, by name.
    #[must_use]
    pub fn tables(&self) -> Vec<String> {
        self.model
            .get("tables")
            .and_then(Value::as_array)
            .map(|tables| {
                tables
                    .iter()
                    .filter_map(|t| t.get("name").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The question that drafts the data model and the ports off the scope.
#[must_use]
pub fn drafts_prompt(c: &Creation) -> String {
    let mut text = String::from(
        "You draft the foundations of an application's proof of concept from its scope, before \
         its mock-up is reviewed. Answer with one JSON object and nothing else:\n\n\
         {\"model\": {\"version\": 1, \"tables\": [{\"name\": \"invoice\", \"purpose\": \"…\", \
         \"business\": [\"a rule on the whole table\"], \"fields\": [{\"name\": \"id\", \"type\": \
         \"uuid\", \"nullable\": false, \"purpose\": \"…\", \"business\": [], \"references\": \
         \"other_table.id\"}]}]},\n \"ports\": [{\"id\": \"billing.listInvoices\", \"kind\": \
         \"capability\", \"purpose\": \"…\"}, {\"id\": \"billing.payInvoice\", \"kind\": \
         \"data_capability\", \"purpose\": \"…\"}]}\n\n\
         `model` is the data the scope's `## Data` section keeps, as PostgreSQL tables: \
         snake_case names, a type per field, a purpose for each table and field, the business \
         rules the scope states; `references` only on a foreign key. `ports` are what the \
         journeys need: a `capability` reads and computes, never writes; a `data_capability` is \
         one typed command that writes. Ids are `<system>.<verbThing>`, the system the bounded \
         context it belongs to. Draft only what the scope asks for. Write the purposes in the \
         language of the human's brief.\n\n",
    );
    text.push_str(&product(c));
    let _ = write!(
        text,
        "\nThe proof of concept's scope:\n\n{}\n",
        c.scope.trim()
    );
    text
}

/// The drafts out of the AI's answer: the model kept as written when it
/// lists tables, the ports with an id, each once.
///
/// # Errors
///
/// The answer carries no JSON, or neither a table nor a port.
pub fn parse_drafts(answer: &str) -> Result<Drafts, String> {
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Err("the AI answered no drafts".to_string());
    };
    let object = answer.get(start..=end).unwrap_or_default();
    let mut drafts: Drafts = serde_json::from_str(object)
        .map_err(|e| format!("the AI's drafts are not readable: {e}"))?;
    if !drafts.model.get("tables").is_some_and(Value::is_array) {
        drafts.model = Value::Null;
    }
    let mut ports: Vec<Port> = Vec::new();
    for mut port in drafts.ports {
        port.id = port.id.trim().to_string();
        port.purpose = port.purpose.trim().to_string();
        if port.id.is_empty() || ports.iter().any(|p| p.id == port.id) {
            continue;
        }
        ports.push(port);
    }
    drafts.ports = ports;
    if drafts.tables().is_empty() && drafts.ports.is_empty() {
        return Err("the AI drafted neither a table nor a port".to_string());
    }
    Ok(drafts)
}

/// Where the drafted ports and the mock-up's micro-frontends disagree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Gaps {
    /// Drafted from the journeys, read or triggered by no micro-frontend.
    pub unused: Vec<String>,
    /// Read or triggered by a micro-frontend, drafted by no journey.
    pub undrafted: Vec<String>,
}

/// The drafted ports set beside the micro-frontends' `needs` and `actions`.
#[must_use]
pub fn gaps(ports: &[Port], components: &[ComponentRef]) -> Gaps {
    let mut used: Vec<&str> = Vec::new();
    for c in components {
        for id in c.needs.iter().chain(c.actions.iter()) {
            if !used.contains(&id.as_str()) {
                used.push(id);
            }
        }
    }
    Gaps {
        unused: ports
            .iter()
            .filter(|p| !used.contains(&p.id.as_str()))
            .map(|p| p.id.clone())
            .collect(),
        undrafted: used
            .into_iter()
            .filter(|id| !ports.iter().any(|p| p.id == *id))
            .map(ToString::to_string)
            .collect(),
    }
}

/// The roadmap task's section on the drafts: the tables, the ports, and
/// where the mock-up disagrees.
#[must_use]
pub fn drafts_section(drafts: &Drafts, components: &[ComponentRef]) -> String {
    let mut body = String::from(
        "\n## Foundations drafted from the scope\n\nDrafts to confirm, not decisions: the data \
         model in contract M's shape and the ports the journeys need. The whole drafts are in \
         the record.\n\n",
    );
    let tables = drafts.tables();
    if !tables.is_empty() {
        let _ = writeln!(body, "- Tables: {}", tables.join(", "));
    }
    for port in &drafts.ports {
        let kind = match port.kind {
            PortKind::Capability => "Capability",
            PortKind::DataCapability => "DataCapability",
        };
        let _ = write!(body, "- {kind} `{}`", port.id);
        if !port.purpose.is_empty() {
            let _ = write!(body, " — {}", port.purpose);
        }
        body.push('\n');
    }
    let gaps = gaps(&drafts.ports, components);
    if !gaps.unused.is_empty() {
        let _ = writeln!(
            body,
            "- Drafted, used by no micro-frontend: {}",
            gaps.unused.join(", ")
        );
    }
    if !gaps.undrafted.is_empty() {
        let _ = writeln!(
            body,
            "- Used by a micro-frontend, drafted by no journey: {}",
            gaps.undrafted.join(", ")
        );
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = "Here.\n```json\n{\"model\": {\"version\": 1, \"tables\": [{\"name\": \"invoice\", \
        \"purpose\": \"a bill\"}, {\"name\": \"payment\"}]}, \"ports\": [{\"id\": \" billing.listInvoices \", \
        \"kind\": \"capability\", \"purpose\": \" the invoices \"}, {\"id\": \"billing.payInvoice\", \
        \"kind\": \"data_capability\"}, {\"id\": \"billing.listInvoices\"}, {\"id\": \"\"}]}\n```";

    fn component(needs: &[&str], actions: &[&str]) -> ComponentRef {
        ComponentRef {
            name: "InvoiceList".into(),
            system: "billing".into(),
            description: String::new(),
            needs: needs.iter().map(|s| (*s).to_string()).collect(),
            actions: actions.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn the_drafts_are_read_and_their_ports_kept_once_each() {
        let drafts = parse_drafts(ANSWER).expect("drafts");
        assert_eq!(drafts.tables(), ["invoice", "payment"]);
        let ids: Vec<&str> = drafts.ports.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["billing.listInvoices", "billing.payInvoice"]);
        assert_eq!(drafts.ports[0].purpose, "the invoices");
        assert_eq!(drafts.ports[1].kind, PortKind::DataCapability);
        assert!(parse_drafts("nothing").is_err());
        assert!(parse_drafts("{oops}").is_err());
        assert!(parse_drafts("{\"model\": {}, \"ports\": []}").is_err());
        let ports_only =
            parse_drafts("{\"model\": \"no\", \"ports\": [{\"id\": \"a.b\"}]}").expect("ports");
        assert_eq!(ports_only.model, Value::Null);
        assert_eq!(ports_only.tables(), Vec::<String>::new());
        let tables_only =
            parse_drafts("{\"model\": {\"tables\": [{\"name\": \"t\"}]}}").expect("tables");
        assert_eq!(tables_only.tables(), ["t"]);
    }

    #[test]
    fn the_gaps_name_what_each_side_lacks() {
        let drafts = parse_drafts(ANSWER).expect("drafts");
        let components = [
            component(&["billing.listInvoices", "billing.totals"], &[]),
            component(&["billing.totals"], &[]),
        ];
        let found = gaps(&drafts.ports, &components);
        assert_eq!(found.unused, ["billing.payInvoice"]);
        assert_eq!(found.undrafted, ["billing.totals"]);
        let agree = gaps(
            &drafts.ports,
            &[component(
                &["billing.listInvoices"],
                &["billing.payInvoice"],
            )],
        );
        assert_eq!(agree, Gaps::default());
    }

    #[test]
    fn the_prompt_asks_for_contract_m_and_the_ports_off_the_scope() {
        let c = Creation {
            name: "bills".into(),
            brief: "Pay invoices".into(),
            scope: "## Data\n\n- invoices".into(),
            ..Creation::default()
        };
        let text = drafts_prompt(&c);
        assert!(text.contains("\"model\""));
        assert!(text.contains("data_capability"));
        assert!(text.contains("## Data\n\n- invoices"));
        assert!(text.contains("Pay invoices"));
    }

    #[test]
    fn the_task_section_lists_tables_ports_and_gaps() {
        let drafts = parse_drafts(ANSWER).expect("drafts");
        let body = drafts_section(&drafts, &[component(&["billing.totals"], &[])]);
        assert!(body.contains("- Tables: invoice, payment"));
        assert!(body.contains("- Capability `billing.listInvoices` — the invoices"));
        assert!(body.contains("- DataCapability `billing.payInvoice`\n"));
        assert!(
            body.contains("used by no micro-frontend: billing.listInvoices, billing.payInvoice")
        );
        assert!(body.contains("drafted by no journey: billing.totals"));
        let quiet = drafts_section(
            &Drafts {
                model: Value::Null,
                ports: Vec::new(),
            },
            &[],
        );
        assert!(!quiet.contains("Tables"));
        assert!(!quiet.contains("used by no"));
    }
}
