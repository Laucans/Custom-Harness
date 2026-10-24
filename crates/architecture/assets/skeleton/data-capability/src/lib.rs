//! DataCapability `{{id}}` (`data-capability.json`): SCAFFOLD: what the
//! command writes, for whom, and which invariants of {{Agg}} it keeps — the
//! SPEC's goal and business rules, in a few sentences.
//!
//! The engine writes one partition per command, `{{Agg}}/<id>`, and judges
//! the row it leaves. [`request`] reads the wire payload, [`prepare`] turns
//! the request and the rows it reads into the row written, or into the
//! [`Refusal`] that says why not — before anything is queued.

use std::collections::BTreeMap;

use async_trait::async_trait;
use harness_dataguard::plugins::{self, Context, Plugin, Prepare};
use serde_json::Value;
use uuid::Uuid;

/// This DataCapability's id, as `data-capability.json` names it.
pub const DATA_CAPABILITY: &str = "{{id}}";

/// The aggregate it writes.
const AGGREGATE: &str = "{{Agg}}";

/// One partition's row, as the engine holds it.
pub type Row = BTreeMap<String, Value>;

/// What the command asks for, read from the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The row it writes, lowercase and hyphenated.
    pub id: String,
    // SCAFFOLD: one field per entry of the manifest's `payload`.
}

/// Why the command is refused before it is queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A field that must be a uuid is not one.
    NotAnId(&'static str),
    /// The row it names does not exist.
    Unknown,
    /// SCAFFOLD: the refusals the SPEC's rules name, one variant each, in
    /// place of this one.
    Unwritten,
}

impl Refusal {
    /// Whether the refusal is of something the request names that does not
    /// exist (404) rather than of a request that cannot be done (422).
    #[must_use]
    pub const fn not_found(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnId(field) => write!(f, "{field}: SCAFFOLD: not an id, in words"),
            Self::Unknown => f.write_str("SCAFFOLD: what is not there, in words"),
            Self::Unwritten => f.write_str("SCAFFOLD: not written yet"),
        }
    }
}

/// What the command becomes before it is submitted: the partition it
/// writes and the fields it writes there — the manifest's `touches`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    /// The row's id: the partition the command lands on.
    pub target_id: String,
    /// The fields written.
    pub payload: Row,
}

/// What the DataCapability plugs into the engine: its `Prepare`.
#[must_use]
pub fn plugin() -> Plugin {
    Plugin::new().prepare(DATA_CAPABILITY, Prepares)
}

/// The command `wire` asks for on `target`, the row the command names.
///
/// # Errors
///
/// [`Refusal::NotAnId`] for an id that is no uuid. SCAFFOLD: every payload
/// field read, each refusal it may answer listed here.
pub fn request(wire: &Row, target: Option<&str>) -> Result<Request, Refusal> {
    let id = target
        .or_else(|| wire.get("id").and_then(Value::as_str))
        .and_then(uuid)
        .ok_or(Refusal::NotAnId("id"))?;
    Ok(Request { id })
}

/// The row `request` writes, `row` being the row it names as the queue
/// leaves it.
///
/// # Errors
///
/// [`Refusal::Unknown`] when the row is not there. SCAFFOLD: the SPEC's
/// business rules, each refusal it may answer listed here.
pub fn prepare(request: &Request, row: &Row) -> Result<Prepared, Refusal> {
    if row.is_empty() {
        return Err(Refusal::Unknown);
    }
    // SCAFFOLD: the fields written, from `request` — the manifest's
    // `touches` and nothing else.
    let _ = &request.id;
    Err(Refusal::Unwritten)
}

/// `text` as a uuid, lowercase and hyphenated as the tables write it; `None`
/// when it is no uuid.
fn uuid(text: &str) -> Option<String> {
    Uuid::parse_str(text.trim()).ok().map(|id| id.to_string())
}

/// The command made the row it writes ([`prepare`]), judged against the
/// rows it reads as the queue leaves them: refused 404 for what is not
/// there, 422 otherwise — nothing queued.
struct Prepares;

#[async_trait]
impl Prepare for Prepares {
    async fn prepare(
        &self,
        context: &Context<'_>,
        payload: &BTreeMap<String, Value>,
        target_id: Option<&str>,
    ) -> Result<plugins::Prepared, plugins::Refusal> {
        let request = request(payload, target_id).map_err(refused)?;
        // SCAFFOLD: the rows it reads — a row as the queue leaves it through
        // `context.row`, SQL through `context.rows` (a `const` with bound
        // `$n`, never filtered again in Rust), the time from
        // `context.now_millis` and never the system clock.
        let row = context.row(&format!("{AGGREGATE}/{}", request.id)).await?;
        let prepared = prepare(&request, &row).map_err(refused)?;
        Ok(plugins::Prepared {
            target_id: Some(prepared.target_id),
            payload: prepared.payload,
            ..plugins::Prepared::default()
        })
    }
}

/// The engine's refusal of `refusal`: 404 for what is not there, 422
/// otherwise.
fn refused(refusal: Refusal) -> plugins::Refusal {
    if refusal.not_found() {
        plugins::Refusal::not_found(refusal.to_string())
    } else {
        plugins::Refusal::unprocessable(refusal.to_string())
    }
}
