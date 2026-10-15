//! The registry of persisted queries (contract C): what the read side
//! serves, and who reads which field — the Resolver's structural
//! dependencies.

use serde::{Deserialize, Serialize};

/// One registered query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registered {
    /// The operation name.
    pub query: String,
    /// The Capability that reads through it.
    pub capability: String,
    /// The file.
    pub file: String,
    /// Its hash, as `scripts/queries.sh` wrote it.
    pub sha256: String,
    /// Every `<entity>.<field>` it reads.
    pub fields: Vec<String>,
}

/// The registry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    /// Every query.
    pub queries: Vec<Registered>,
}

impl Registry {
    /// Reads `queries/registry.json`.
    ///
    /// # Errors
    ///
    /// The text is not the contract's JSON.
    pub fn parse(json: &str) -> std::result::Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The query `capability` reads through under this hash, if registered:
    /// what the read crate serves, and nothing else.
    #[must_use]
    pub fn served(&self, capability: &str, sha256: &str) -> Option<&Registered> {
        self.queries
            .iter()
            .find(|q| q.capability == capability && q.sha256 == sha256)
    }

    /// The Capabilities that read any field of `aggregate` — or of one
    /// field, when `field` names it: `Account.creditLimit`. Sorted, each
    /// once.
    #[must_use]
    pub fn readers_of(&self, aggregate: &str, field: Option<&str>) -> Vec<String> {
        let prefix = format!("{aggregate}.");
        let mut readers: Vec<String> = self
            .queries
            .iter()
            .filter(|q| {
                q.fields.iter().any(|f| {
                    field.map_or_else(
                        || f.starts_with(&prefix) || f.eq_ignore_ascii_case(aggregate),
                        |wanted| f.eq_ignore_ascii_case(wanted),
                    )
                })
            })
            .map(|q| q.capability.clone())
            .collect();
        readers.sort();
        readers.dedup();
        readers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::parse(
            r#"{"queries":[
              {"query":"CustomerPayments","capability":"customer.calculateRisk","file":"a.graphql","sha256":"aa","fields":["Customer.outstandingBalance","Payment.dueAt"]},
              {"query":"Invoices","capability":"billing.invoiceList","file":"b.graphql","sha256":"bb","fields":["Payment.amount"]},
              {"query":"Timeline","capability":"crm.timeline","file":"c.graphql","sha256":"cc","fields":["Customer.name","Payment.dueAt"]}
            ]}"#,
        )
        .expect("contract C")
    }

    #[test]
    fn a_query_is_served_under_its_capability_and_hash_only() {
        let registry = registry();
        assert!(registry.served("customer.calculateRisk", "aa").is_some());
        assert!(
            registry.served("customer.calculateRisk", "bb").is_none(),
            "another hash"
        );
        assert!(
            registry.served("crm.timeline", "aa").is_none(),
            "another Capability"
        );
    }

    #[test]
    fn the_readers_of_an_aggregate_or_a_field_are_listed_once_and_sorted() {
        let registry = registry();
        assert_eq!(
            registry.readers_of("Payment", None),
            [
                "billing.invoiceList",
                "crm.timeline",
                "customer.calculateRisk"
            ]
        );
        assert_eq!(
            registry.readers_of("Payment", Some("Payment.dueAt")),
            ["crm.timeline", "customer.calculateRisk"]
        );
        assert!(
            registry.readers_of("Nobody", None).is_empty(),
            "expected nothing"
        );
        assert!(
            Registry::default().readers_of("Payment", None).is_empty(),
            "expected nothing"
        );
    }
}
