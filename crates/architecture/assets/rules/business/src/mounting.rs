//! How a rule's test is mounted, checked at its first real run — once the
//! units it needs are built and their manifests exist:
//!
//! - every unit it reaches is named in its needs;
//! - a read sends only the variables its `capability.json` `input` takes;
//! - a command sends only the payload keys its `data-capability.json`
//!   declares — the `DataGuard` engine's own judgement, read back from the
//!   answer's `warnings`, never a grammar of the kit's own.
//!
//! A violation is the test's, never the product's: the rule is
//! **mis-mounted** — neither red nor contested — and the rules' task fixes
//! the test. Values are never typed: a fixture's id where a manifest says
//! `uuid` is a value, and accepted. `std` and `serde_json` only.

use serde_json::{Map, Value};

/// An id without its version: `campaign.saveEntity@2` → `campaign.saveEntity`.
#[must_use]
pub fn unversioned(id: &str) -> &str {
    id.split_once('@').map_or(id, |(name, _)| name)
}

/// What is wrong when a test reaches `kind:id` and its needs do not name it.
#[must_use]
pub fn unnamed_need(needs: &[String], kind: &str, id: &str) -> Option<String> {
    let id = unversioned(id);
    let named = needs.iter().any(|need| {
        need.split_once(':')
            .is_some_and(|(k, named)| k == kind && unversioned(named) == id)
    });
    (!named).then(|| {
        format!("reaches {kind}:{id}, which its needs do not name: a test lists every unit it uses")
    })
}

/// Whether `declared` takes the wire key `key`: by name, or through a dotted
/// path under it (`entity.id` declares `entity`), as the engine reads it.
fn declares(declared: &Map<String, Value>, key: &str) -> bool {
    declared.contains_key(key)
        || declared.keys().any(|name| {
            name.strip_prefix(key)
                .is_some_and(|rest| rest.starts_with('.'))
        })
}

/// What is wrong when a read of `capability` sends a variable its manifest's
/// `input` does not take; nothing when the manifest gives no `input` object.
/// `path` is the manifest's, as the line names it.
#[must_use]
pub fn off_input(
    capability: &str,
    variables: &Value,
    manifest: &Value,
    path: &str,
) -> Option<String> {
    let input = manifest.get("input")?.as_object()?;
    let sent = variables.as_object()?;
    let off: Vec<String> = sent
        .keys()
        .filter(|key| !declares(input, key))
        .map(|key| format!("\"{key}\""))
        .collect();
    if off.is_empty() {
        return None;
    }
    let takes = if input.is_empty() {
        "takes no input".to_string()
    } else {
        format!(
            "takes {}",
            input.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    };
    Some(format!(
        "{capability} has no input {}; its capability.json {takes} ({path})",
        off.join(", ")
    ))
}

/// The engine's undeclared-key warnings in what a `POST /commands`
/// answered — its own `warnings`, never a follow-up's:
/// `payload key(s) <keys> not declared by <dataCapability>`.
#[must_use]
pub fn off_payload(answer: &Value) -> Vec<String> {
    answer
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|w| w.starts_with("payload key(s) ") && w.contains(" not declared by "))
        .map(|w| format!("sends a payload off its manifest: {w}"))
        .collect()
}
