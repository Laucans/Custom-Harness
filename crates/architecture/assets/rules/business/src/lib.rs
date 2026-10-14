//! The rules' kit, for the business rules' tests (`tests/*.rs`): a test
//! proves one rule of `rules.json` (contract P), written before the code it
//! proves, and so it reaches the product only the way a Micro-UI does — by
//! identifier, over the shell's wire protocol:
//!
//! - `POST /capabilities/<id>` with `{ "variables": … }` reads a Capability;
//! - `POST /commands` with `{ "dataCapability": "<name>@<v>", "payload": …,
//!   "target": { "id": … } }` sends a command, its result (contract H) read
//!   back at `GET /commands/<id>` until it settles.
//!
//! A rule's test is **pending** while a unit it needs is not built yet (no
//! manifest names it) or no server answers (`RULES_SERVER_URL` unset): it
//! passes, and says so. Once everything it needs exists it runs, and must
//! hold. Each outcome is written, one JSON line, to `RULES_STATUS` (default
//! `target/rules-status.jsonl`), which `scripts/rules-gate.mjs` reads.
//!
//! ```ignore
//! use rules_business::{rule, fixture};
//!
//! #[test]
//! fn r3_a_campaign_archived_leaves_the_list() {
//!     // given: a campaign in the list
//!     // when: the DM archives it
//!     // then: it is no longer listed
//!     rule("R3", &["data-capability:ArchiveCampaign", "capability:ListCampaigns"], |s| {
//!         let campaign = fixture("campaigns")[0].clone();
//!         s.command("ArchiveCampaign@1", serde_json::json!({}), campaign["id"].as_str())?;
//!         let listed = s.read("ListCampaigns", serde_json::json!({}))?;
//!         rules_business::check(!listed.to_string().contains(campaign["id"].as_str().unwrap_or("")), "still listed")
//!     });
//! }
//! ```

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// How long a command may take to settle before the test says so.
const SETTLE: Duration = Duration::from_secs(20);

/// The repository's root: this crate sits at `tests/rules/business`.
#[must_use]
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The product's server, as the shell's wire protocol reaches it.
pub struct Server {
    base: String,
}

impl Server {
    /// Reads a Capability by its identifier; answers its `data`.
    ///
    /// # Errors
    ///
    /// The server refused it or answered no data.
    pub fn read(&self, capability: &str, variables: Value) -> Result<Value, String> {
        let url = format!("{}/capabilities/{capability}", self.base);
        let answer: Value = ureq::post(&url)
            .send_json(json!({ "variables": variables }))
            .map_err(|e| format!("reading {capability}: {e}"))?
            .into_json()
            .map_err(|e| format!("reading {capability}: {e}"))?;
        Ok(answer.get("data").cloned().unwrap_or(Value::Null))
    }

    /// Sends a command to a DataCapability (`<name>@<version>`, the version
    /// 1 when absent) and waits for its result (contract H) to settle.
    ///
    /// # Errors
    ///
    /// The server refused it, or it did not settle in time.
    pub fn command(
        &self,
        data_capability: &str,
        payload: Value,
        target: Option<&str>,
    ) -> Result<Value, String> {
        let name = if data_capability.contains('@') {
            data_capability.to_string()
        } else {
            format!("{data_capability}@1")
        };
        let mut body = json!({ "dataCapability": name, "payload": payload });
        if let Some(id) = target {
            body["target"] = json!({ "id": id });
        }
        let accepted: Value = ureq::post(&format!("{}/commands", self.base))
            .send_json(body)
            .map_err(|e| format!("commanding {data_capability}: {e}"))?
            .into_json()
            .map_err(|e| format!("commanding {data_capability}: {e}"))?;
        let mut result = accepted.get("result").cloned().unwrap_or(Value::Null);
        let id = accepted
            .get("commandId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let started = Instant::now();
        while !settled(&result) {
            if started.elapsed() > SETTLE {
                return Err(format!(
                    "{data_capability}: not settled after {}s",
                    SETTLE.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
            let read: Value = ureq::get(&format!("{}/commands/{id}", self.base))
                .call()
                .map_err(|e| format!("reading the command {id}: {e}"))?
                .into_json()
                .map_err(|e| format!("reading the command {id}: {e}"))?;
            result = read.get("result").cloned().unwrap_or(Value::Null);
        }
        Ok(result)
    }
}

/// A command's result has settled: applied or rejected.
fn settled(result: &Value) -> bool {
    matches!(
        result.get("status").and_then(Value::as_str),
        Some("applied" | "rejected")
    )
}

/// A fixture of the mock-up (`mock/fixtures/<name>.json`): the data the
/// human validated with it, a test's given.
///
/// # Panics
///
/// The fixture is not there or not JSON: the test is wrong, not the product.
#[must_use]
pub fn fixture(name: &str) -> Value {
    let path = root().join("mock/fixtures").join(format!("{name}.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A then that does not hold, said.
///
/// # Errors
///
/// `ok` is false.
pub fn check(ok: bool, why: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(why.to_string()) }
}

/// What a manifest names, by the kind of unit a test may need.
const KINDS: [(&str, &str, &str); 5] = [
    ("capability", "capability.json", "capability"),
    ("data-capability", "data-capability.json", "dataCapability"),
    ("micro-ui", "micro-ui.json", "microUi"),
    ("aggregate", "aggregate.json", "aggregate"),
    ("screen", "", "screen"),
];

/// Every manifest file under a folder.
fn manifests(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "target" || n == "node_modules")
            {
                continue;
            }
            manifests(&path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
}

/// A unit the product has built: a manifest names it. `kind:name`, as a
/// rule's test declares what it needs.
#[must_use]
pub fn built(need: &str) -> bool {
    let Some((kind, name)) = need.split_once(':') else {
        return false;
    };
    let Some((_, file, field)) = KINDS.iter().find(|(k, _, _)| *k == kind) else {
        return false;
    };
    let mut found = Vec::new();
    manifests(&root().join("crates"), &mut found);
    manifests(&root().join("apps"), &mut found);
    found.iter().any(|path| {
        let named = if file.is_empty() {
            path.parent().is_some_and(|p| p.ends_with("apps/screens"))
        } else {
            path.file_name().is_some_and(|n| n == *file)
        };
        named
            && fs::read_to_string(path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .is_some_and(|m| m.get(*field).and_then(Value::as_str) == Some(name))
    })
}

/// Writes one outcome where the rules' gate reads it.
fn record(id: &str, state: &str, why: &str) {
    let test = std::thread::current().name().unwrap_or("?").to_string();
    let path = std::env::var("RULES_STATUS")
        .map_or_else(|_| root().join("target/rules-status.jsonl"), PathBuf::from);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let line =
        json!({ "rule": id, "family": "business", "test": test, "state": state, "why": why });
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
}

/// Proves rule `id`: pending while a unit it `needs` is not built or no
/// server answers; else `body` runs on the server, and must hold.
///
/// # Panics
///
/// The rule does not hold: the test fails, as it must.
pub fn rule(id: &str, needs: &[&str], body: impl FnOnce(&Server) -> Result<(), String>) {
    let missing: Vec<&str> = needs.iter().copied().filter(|n| !built(n)).collect();
    if !missing.is_empty() {
        let why = format!("waiting for {}", missing.join(", "));
        eprintln!("{id} pending: {why}");
        record(id, "pending", &why);
        return;
    }
    let Ok(base) = std::env::var("RULES_SERVER_URL") else {
        let why = "no server: crates/server is not built, or no DATABASE_URL";
        eprintln!("{id} pending: {why}");
        record(id, "pending", why);
        return;
    };
    let server = Server {
        base: base.trim_end_matches('/').to_string(),
    };
    match panic::catch_unwind(AssertUnwindSafe(|| body(&server))) {
        Ok(Ok(())) => record(id, "green", ""),
        Ok(Err(why)) => {
            record(id, "red", &why);
            panic!("{id} does not hold: {why}");
        }
        Err(cause) => {
            let why = cause
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| cause.downcast_ref::<&str>().map(ToString::to_string))
                .unwrap_or_else(|| "it panicked".to_string());
            record(id, "red", &why);
            panic::resume_unwind(cause);
        }
    }
}
