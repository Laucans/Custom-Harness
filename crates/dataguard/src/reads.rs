//! The Capabilities' reads: each [`Answer`] a plugin brings, run on demand.
//!
//! A read runs the Capability's query over a connection that only reads,
//! then hands its rows and the request's variables to the Capability's
//! answer. In memory there is no database: the rows are `[]`, and the
//! answer is what the Capability makes of nothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::plugins::{Answer, Plugins};
use crate::{Error, Result};

/// What runs a Capability's query and hands its rows back.
#[async_trait]
pub trait Rows: Send + Sync {
    /// The rows `sql` answers with these parameters, as a JSON array.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the database does not answer.
    async fn rows(&self, sql: &str, parameters: &[Value]) -> Result<Value>;
}

/// Every Capability's answer, by Capability.
#[derive(Default)]
pub struct Reads {
    answers: BTreeMap<String, Answer>,
    connection: Option<Arc<dyn Rows>>,
}

impl From<&Plugins> for Reads {
    fn from(plugins: &Plugins) -> Self {
        Self {
            answers: plugins
                .0
                .answers
                .iter()
                .map(|answer| (answer.capability.clone(), answer.clone()))
                .collect(),
            connection: None,
        }
    }
}

impl Reads {
    /// The reads over a database, in place of none.
    #[must_use]
    pub fn with_connection(mut self, connection: Arc<dyn Rows>) -> Self {
        self.connection = Some(connection);
        self
    }

    /// The answer of `capability` to these variables: its query's rows —
    /// `[]` without a database — handed to its answer. A variable the
    /// query binds and the request lacks is bound as `null`.
    ///
    /// # Errors
    ///
    /// [`Error::UnregisteredQuery`] for a Capability no plugin answers, or
    /// the connection's.
    pub async fn run(
        &self,
        capability: &str,
        variables: &BTreeMap<String, Value>,
    ) -> Result<Value> {
        let answer = self
            .answers
            .get(capability)
            .ok_or_else(|| Error::UnregisteredQuery(capability.to_string()))?;
        let rows = match &self.connection {
            Some(connection) => {
                let parameters: Vec<Value> = answer
                    .variables
                    .iter()
                    .map(|name| variables.get(name).cloned().unwrap_or(Value::Null))
                    .collect();
                connection.rows(&answer.sql, &parameters).await?
            }
            None => Value::Array(Vec::new()),
        };
        Ok((answer.answer)(&rows, variables))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::plugins::Plugin;

    /// Counts the rows, and echoes the account asked for.
    fn risk(rows: &Value, variables: &BTreeMap<String, Value>) -> Value {
        json!({
            "rows": rows.as_array().map_or(0, Vec::len),
            "accountId": variables.get("accountId").cloned().unwrap_or(Value::Null),
        })
    }

    fn plugins() -> Plugins {
        std::iter::once(Plugin::new().answer(Answer {
            capability: "credit.calculateRisk".to_string(),
            sql: "select * from risk where account = $1 and day = $2".to_string(),
            variables: vec!["accountId".to_string(), "day".to_string()],
            answer: risk,
        }))
        .collect()
    }

    /// A database that keeps what it was asked and answers two rows.
    #[derive(Default)]
    struct Asked(Mutex<Vec<(String, Vec<Value>)>>);

    #[async_trait]
    impl Rows for Asked {
        async fn rows(&self, sql: &str, parameters: &[Value]) -> Result<Value> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((sql.to_string(), parameters.to_vec()));
            Ok(json!([{ "score": 1 }, { "score": 2 }]))
        }
    }

    #[tokio::test]
    async fn an_answer_over_memory_is_made_of_no_rows() {
        let reads = Reads::from(&plugins());
        let variables = BTreeMap::from([("accountId".to_string(), json!("881"))]);
        let answer = reads
            .run("credit.calculateRisk", &variables)
            .await
            .expect("answered");
        assert_eq!(answer, json!({ "rows": 0, "accountId": "881" }));
    }

    #[tokio::test]
    async fn a_capability_no_plugin_answers_is_unregistered() {
        let reads = Reads::from(&plugins());
        let refused = reads.run("credit.nothing", &BTreeMap::new()).await;
        assert_eq!(
            refused,
            Err(Error::UnregisteredQuery("credit.nothing".to_string()))
        );
        assert!(
            Reads::default()
                .run("credit.calculateRisk", &BTreeMap::new())
                .await
                .is_err(),
            "no plugin, no answer"
        );
    }

    #[tokio::test]
    async fn over_a_connection_the_variables_are_bound_in_order_and_the_rows_answered() {
        let asked = Arc::new(Asked::default());
        let reads = Reads::from(&plugins()).with_connection(asked.clone());
        let variables = BTreeMap::from([("accountId".to_string(), json!("881"))]);
        let answer = reads
            .run("credit.calculateRisk", &variables)
            .await
            .expect("answered");
        assert_eq!(answer, json!({ "rows": 2, "accountId": "881" }));
        let asked = asked
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(
            asked,
            [(
                "select * from risk where account = $1 and day = $2".to_string(),
                vec![json!("881"), Value::Null]
            )],
            "the query's own variables, in its order, a missing one null"
        );
    }
}
