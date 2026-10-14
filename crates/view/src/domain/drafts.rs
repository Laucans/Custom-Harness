//! A page's **draft**: what the human changed on the mock-up's page in the
//! view's edit mode, kept as **operations** on the page as committed — a
//! part moved before another, resized to a fraction of its row, removed,
//! cloned, its text rewritten, a container laid out freely, a link or a
//! button added, a micro-frontend forked — never as another copy of the
//! HTML. Each operation says who made it: the human, or the request it
//! came from (`by`).
//!
//! Drafts come in **versions**, per board (a branch and a page). Opening the
//! page continues the open version, or starts a new one from the latest
//! when the last was closed; closing — leaving the page, or *keep* — closes
//! it; *restore* copies an older version in front; *validate* marks a
//! closed version as the next round of development. The newest
//! [`KEPT`] closed versions survive beside the open one, validated ones
//! always. Undo lives in the page, forty operations deep, and ends with the
//! version.
//!
//! A version remembers the page it was made on (`base`, the page file's
//! fingerprint); the page moved under it is a **frozen** draft, which the
//! page shows as such and offers to restart from the page as it is.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Write as _;

/// How many closed versions a board keeps beside the open one.
pub const KEPT: usize = 3;

/// How many operations a version may hold.
pub const MAX_OPS: usize = 2_000;

/// The operations a draft knows.
pub const KINDS: [&str; 10] = [
    "move", "resize", "remove", "clone", "text", "static", "add", "fork", "viewport", "props",
];

/// Where a version stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Being edited: the one the page writes to.
    #[default]
    Open,
    /// Closed: kept in the history, read only.
    Kept,
    /// Closed and marked as the next round of development.
    Validated,
}

/// One version of a page's draft.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// 1, 2, 3… per board.
    pub version: u32,
    /// A name the human gave it, or empty.
    #[serde(default)]
    pub name: String,
    /// Where it stands.
    #[serde(default)]
    pub state: State,
    /// The fingerprint of the page it was made on.
    #[serde(default)]
    pub base: String,
    /// The operations, oldest first.
    #[serde(default)]
    pub ops: Vec<Value>,
    /// The width the page was edited at.
    #[serde(default)]
    pub viewport: u32,
    /// When it was opened, as the page's clock says.
    #[serde(default)]
    pub created_at: String,
    /// When it was closed; empty while open.
    #[serde(default)]
    pub closed_at: String,
}

/// A board's drafts as the page reads them: the open one, and the closed
/// ones newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct History {
    /// The version being edited, if any.
    pub current: Option<Draft>,
    /// The closed versions, newest first.
    pub kept: Vec<Draft>,
}

/// A board's versions sorted into a history.
#[must_use]
pub fn history(mut versions: Vec<Draft>) -> History {
    versions.sort_by_key(|d| std::cmp::Reverse(d.version));
    let current = versions.iter().find(|d| d.state == State::Open).cloned();
    let kept = versions
        .into_iter()
        .filter(|d| d.state != State::Open)
        .collect();
    History { current, kept }
}

/// Checks the operations a page sends: a bounded list of objects, each of a
/// known kind on a named target, signed by the human or a request.
///
/// # Errors
///
/// The sentence the page shows.
pub fn checked_ops(ops: &[Value]) -> Result<(), String> {
    if ops.len() > MAX_OPS {
        return Err(format!("a draft holds at most {MAX_OPS} operations"));
    }
    for (i, op) in ops.iter().enumerate() {
        let Some(object) = op.as_object() else {
            return Err(format!("operation {i}: not an object"));
        };
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !KINDS.contains(&kind) {
            return Err(format!("operation {i}: unknown kind {kind:?}"));
        }
        let target = object
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if target.chars().count() > 300 {
            return Err(format!("operation {i}: its target is named too long"));
        }
        let by = object.get("by").and_then(Value::as_str).unwrap_or_default();
        if by.is_empty() || by.chars().count() > 60 {
            return Err(format!("operation {i}: says not who made it"));
        }
    }
    Ok(())
}

/// What opening the page does to a board's versions: continue the open
/// one, or start a new one from the latest — its operations copied when
/// the page is still the one they were made on, dropped otherwise. Answers
/// the version to write, when one must be.
#[must_use]
pub fn opened(versions: &[Draft], base: &str, now: &str) -> Option<Draft> {
    if versions.iter().any(|d| d.state == State::Open) {
        return None;
    }
    let latest = versions.iter().max_by_key(|d| d.version);
    let next = latest.map_or(1, |d| d.version.saturating_add(1));
    let ops = latest
        .filter(|d| d.base == base)
        .map(|d| d.ops.clone())
        .unwrap_or_default();
    Some(Draft {
        version: next,
        name: String::new(),
        state: State::Open,
        base: base.to_string(),
        ops,
        viewport: latest.map_or(0, |d| d.viewport),
        created_at: now.to_string(),
        closed_at: String::new(),
    })
}

/// A version restored: a new open one with its operations, in front.
///
/// # Errors
///
/// No such version.
pub fn restored(versions: &[Draft], version: u32, now: &str) -> Result<Draft, String> {
    let from = versions
        .iter()
        .find(|d| d.version == version)
        .ok_or_else(|| format!("no version {version}"))?;
    let next = versions
        .iter()
        .map(|d| d.version)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    Ok(Draft {
        version: next,
        name: from.name.clone(),
        state: State::Open,
        base: from.base.clone(),
        ops: from.ops.clone(),
        viewport: from.viewport,
        created_at: now.to_string(),
        closed_at: String::new(),
    })
}

/// The versions to destroy once a board has more closed ones than it keeps:
/// the oldest beyond [`KEPT`], validated ones never.
#[must_use]
pub fn beyond_kept(versions: &[Draft]) -> Vec<u32> {
    let mut closed: Vec<&Draft> = versions.iter().filter(|d| d.state == State::Kept).collect();
    closed.sort_by_key(|d| std::cmp::Reverse(d.version));
    closed.into_iter().skip(KEPT).map(|d| d.version).collect()
}

/// A fingerprint of a file's bytes: FNV-1a 64, sixteen hex digits — what a
/// version remembers of the page it was made on.
#[must_use]
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// One request to apply to the draft, as the page sends it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asked {
    /// The pin's id — what the operations it produces are signed with.
    pub id: String,
    /// The intention, the human's words.
    pub text: String,
    /// What the pin sits on, in the page's words (`component:RiskBadge`,
    /// `id:create`), or empty for the page.
    #[serde(default)]
    pub target: String,
    /// What the pin points at when it is rendered at run time inside the
    /// target (a kit component's insides) and so not in the page's source;
    /// empty when the target is the part itself.
    #[serde(default)]
    pub inside: String,
    /// Where the pin or the zone stands: its place in the target and the
    /// elements around it, or the elements a zone covers.
    #[serde(default)]
    pub place: String,
}

/// The prompt that applies requests to a page's draft in one go: the page
/// as the draft shows it, what can be done to it, the requests, and the
/// shape of the answer — a JSON list of operations, each signed with the
/// request it answers.
#[must_use]
pub fn apply_prompt(
    page: &str,
    html: &str,
    components: &str,
    kit: &str,
    asked: &[Asked],
) -> String {
    let requests = asked
        .iter()
        .map(|a| {
            let mut line = format!(
                "- request `{}`{}: {}",
                a.id,
                if a.target.is_empty() {
                    String::new()
                } else {
                    format!(" (on {})", a.target)
                },
                a.text.trim()
            );
            if !a.inside.trim().is_empty() {
                let _ = write!(
                    line,
                    "\n  it points inside that element, at a part rendered at run time and not in the page's source: {}",
                    a.inside.trim()
                );
            }
            if !a.place.trim().is_empty() {
                let _ = write!(line, "\n  where: {}", a.place.trim());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You edit the mock-up of a product's page, `{page}`, on behalf of a human who dropped \
         requests on it. The page below is static HTML plus micro-frontends (elements with \
         `data-component`, whose rendered content is shown but belongs to the micro-frontend's own \
         code). You answer ONLY a JSON array of operations on that page, nothing else — no prose, \
         no code fence. Each operation is an object with `kind`, `by` (the request's id), a \
         `target` naming an existing element by `component:<Name>`, `kit:<Name>` (a kit \
         component; `kit:<Name>@<n>` for the n-th of that name on the page), `id:<id>` or \
         `data:<data-id>`, and the fields of its kind:\n\
         - `move`: `before` or `after` (a target) or `into` (a container target) — reorders, never \
         positions freely;\n\
         - `resize`: `cols` 1..12, the width as twelfths of the row;\n\
         - `remove`;\n\
         - `clone`: copies the target right after itself, `as` the new element's id;\n\
         - `text`: `text`, the new text of a static element;\n\
         - `static`: `layout` `static`|`flow` on a container;\n\
         - `add`: `what` `link`|`button`|`text`|`section`|`image`|`kit`|`component`, `label`, and \
         `page` (a page path) or `href` for a link, `intent` (what the button is for, in words) for \
         a button, `name` (a component of the kit, listed below) for `kit` with optional `props` (an \
         object), `name` (a micro-frontend of the branch, listed below) for `component` — the \
         micro-frontend injected on the page, as its own code draws it; placed \
         `before`/`after`/`into` like a move, with `as` the new element's id;\n\
         - `props`: `props`, an object, on a kit component already on the page.\n\
         The product has a **design system**: a kit of styled visual components (a menu, a \
         carousel, a card…) listed below. When a request calls for something visual the kit \
         holds, add the kit component (`what: kit`) rather than inventing markup. When it calls \
         for a visual component the kit lacks, do not invent it: answer for that request one \
         operation `{{\"kind\":\"add\",\"by\":\"<id>\",\"skip\":\"missing component: <Name> — <what it is, one line>\"}}` \
         and the human decides (build it, edit one, a placeholder, or without).\n\
         A request that changes what a micro-frontend does inside: do the page's part yourself \
         (injecting it with `add` `what: component`, placing it, what goes around it), and for its \
         inside answer one operation `{{\"kind\":\"mf\",\"by\":\"<id>\",\"name\":\"<Name>\",\"ask\":\"<what \
         must change inside it, in the human's words>\"}}` per micro-frontend, naming only those \
         listed below: each is redrafted from it apart. Answer \
         `{{\"kind\":\"add\",\"by\":\"<id>\",\"skip\":\"<why, one line>\"}}` only for what neither the page \
         nor a listed micro-frontend can carry. A kit component's insides are rendered at run time: act on \
         the kit component itself (its `props`, or what goes before, after or around it). A \
         request's `where` says where the human pointed — use it to choose `before`, `after` or \
         `into`. Use only elements that exist in the page. Keep the changes \
         small and faithful to each request.\n\n\
         The micro-frontends on this branch:\n{components}\n\n\
         The kit's components:\n{kit}\n\n\
         The requests:\n{requests}\n\n<page>\n{html}\n</page>"
    )
}

/// A request's part that is a micro-frontend's inside, as the answer
/// hands it over: redrafted apart, on that micro-frontend.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MfAsk {
    /// The request it comes from.
    pub by: String,
    /// The micro-frontend.
    pub name: String,
    /// What must change inside it.
    pub ask: String,
}

/// What an answer carries: the page's operations, and the micro-frontends'
/// insides handed over (`kind: mf`), each signed with a request asked.
///
/// # Errors
///
/// No array in the answer, an operation that is not one, or a hand-over
/// with no micro-frontend or nothing to do.
pub fn answered(answer: &str, asked: &[Asked]) -> Result<(Vec<Value>, Vec<MfAsk>), String> {
    let all = ops_from_answer(answer, asked)?;
    let mut ops = Vec::new();
    let mut mfs = Vec::new();
    for op in all {
        if op.get("kind").and_then(Value::as_str) == Some("mf") {
            let mf: MfAsk = serde_json::from_value(op)
                .map_err(|e| format!("a micro-frontend's part is not readable: {e}"))?;
            if mf.name.trim().is_empty() || mf.ask.trim().is_empty() {
                return Err("a micro-frontend's part names none, or asks nothing".to_string());
            }
            mfs.push(mf);
        } else {
            ops.push(op);
        }
    }
    checked_ops(&ops)?;
    Ok((ops, mfs))
}

/// The operations an answer carries: a JSON array, alone or inside a code
/// fence, each operation signed with a request that was asked; a hand-over
/// to a micro-frontend (`kind: mf`) among them.
///
/// # Errors
///
/// No array in the answer, or an operation that is not one.
fn ops_from_answer(answer: &str, asked: &[Asked]) -> Result<Vec<Value>, String> {
    let start = answer.find('[').ok_or("the answer carries no list")?;
    let end = answer.rfind(']').ok_or("the answer carries no list")?;
    if end < start {
        return Err("the answer carries no list".to_string());
    }
    let text = answer.get(start..=end).unwrap_or_default();
    let ops: Vec<Value> = serde_json::from_str(text)
        .map_err(|e| format!("the answer is not a list of operations: {e}"))?;
    let draft: Vec<Value> = ops
        .iter()
        .filter(|o| o.get("kind").and_then(Value::as_str) != Some("mf"))
        .cloned()
        .collect();
    checked_ops(&draft)?;
    for op in &ops {
        let by = op.get("by").and_then(Value::as_str).unwrap_or_default();
        if !asked.iter().any(|a| a.id == by) {
            return Err(format!("an operation answers `{by}`, which was not asked"));
        }
    }
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_says_what_it_points_inside_and_where() {
        let asked = [
            Asked {
                id: "r1".to_string(),
                text: "add the logo here".to_string(),
                target: "kit:Menu".to_string(),
                inside: " inside the kit component Menu, rendered at run time — <nav>…</nav> "
                    .to_string(),
                place: " 10% across and 50% down in kit:Menu, in an empty space before id:search "
                    .to_string(),
            },
            Asked {
                id: "r2".to_string(),
                text: "bigger".to_string(),
                ..Asked::default()
            },
        ];
        let prompt = apply_prompt("pages/a.html", "<main></main>", "", "", &asked);
        assert!(prompt.contains("- request `r1` (on kit:Menu): add the logo here\n  it points inside that element, at a part rendered at run time and not in the page's source: inside the kit component Menu, rendered at run time — <nav>…</nav>\n  where: 10% across and 50% down in kit:Menu, in an empty space before id:search"));
        assert!(
            prompt.contains("- request `r2`: bigger\n"),
            "nothing more said when nothing more is known"
        );
        assert!(prompt.contains("`kit:<Name>` (a kit component; `kit:<Name>@<n>`"));
        assert!(prompt.contains("A kit component's insides are rendered at run time"));
    }
    use serde_json::json;

    fn draft(version: u32, state: State, base: &str) -> Draft {
        Draft {
            version,
            state,
            base: base.into(),
            ops: vec![json!({"kind": "remove", "by": "human", "target": "id:x"})],
            ..Draft::default()
        }
    }

    #[test]
    fn opening_continues_the_open_version_or_starts_one_from_the_latest() {
        assert!(opened(&[draft(1, State::Open, "a")], "a", "t").is_none());
        let next = opened(&[draft(1, State::Kept, "a")], "a", "t").expect("a new one");
        assert_eq!(next.version, 2);
        assert_eq!(next.ops.len(), 1, "the latest's operations carried over");
        let moved = opened(&[draft(1, State::Kept, "a")], "b", "t").expect("a new one");
        assert!(moved.ops.is_empty(), "the page moved: nothing carried over");
        assert_eq!(moved.base, "b");
        let first = opened(&[], "a", "t").expect("a first one");
        assert_eq!(first.version, 1);
    }

    #[test]
    fn restoring_copies_a_version_in_front_and_the_oldest_kept_go() {
        let versions = vec![
            draft(1, State::Kept, "a"),
            draft(2, State::Validated, "a"),
            draft(3, State::Kept, "a"),
            draft(4, State::Kept, "a"),
            draft(5, State::Kept, "a"),
            draft(6, State::Open, "a"),
        ];
        let again = restored(&versions, 2, "t").expect("restored");
        assert_eq!(again.version, 7);
        assert_eq!(again.state, State::Open);
        assert!(restored(&versions, 9, "t").is_err());
        assert_eq!(
            beyond_kept(&versions),
            [1],
            "three kept stay, validated stays"
        );
        let h = history(versions);
        assert_eq!(h.current.map(|d| d.version), Some(6));
        assert_eq!(
            h.kept.iter().map(|d| d.version).collect::<Vec<_>>(),
            [5, 4, 3, 2, 1]
        );
    }

    #[test]
    fn operations_are_checked_and_read_back_from_an_answer() {
        assert!(
            checked_ops(&[
                json!({"kind": "move", "by": "human", "target": "id:a", "before": "id:b"})
            ])
            .is_ok()
        );
        assert!(checked_ops(&[json!({"kind": "teleport", "by": "human"})]).is_err());
        assert!(
            checked_ops(&[json!({"kind": "remove", "target": "id:a"})]).is_err(),
            "unsigned"
        );
        assert!(checked_ops(&[json!("nope")]).is_err());
        let asked = vec![Asked {
            id: "r1".into(),
            text: "add an archive button".into(),
            target: "component:Campagnes".into(),
            ..Asked::default()
        }];
        let answer = "Here you go:\n```json\n[{\"kind\":\"add\",\"by\":\"r1\",\"target\":\"component:Campagnes\",\"what\":\"button\",\"label\":\"Archiver\",\"after\":\"id:create\",\"as\":\"archive\"}]\n```";
        let ops = ops_from_answer(answer, &asked).expect("read");
        assert_eq!(ops.len(), 1);
        let (ops, mfs) = answered(
            r#"[{"kind":"add","by":"r1","what":"component","name":"Rail","after":"id:a","as":"rail"},
                {"kind":"mf","by":"r1","name":"Rail","ask":"a folded mode"}]"#,
            &asked,
        )
        .expect("read");
        assert_eq!(ops.len(), 1, "the page's part");
        assert_eq!(
            mfs,
            [MfAsk {
                by: "r1".into(),
                name: "Rail".into(),
                ask: "a folded mode".into()
            }]
        );
        assert!(answered(r#"[{"kind":"mf","by":"r1","name":"","ask":"x"}]"#, &asked).is_err());
        assert!(
            answered(
                r#"[{"kind":"mf","by":"r9","name":"Rail","ask":"x"}]"#,
                &asked
            )
            .is_err()
        );
        assert!(answered(r#"[{"kind":"teleport","by":"r1"}]"#, &asked).is_err());
        assert!(
            ops_from_answer(
                "[{\"kind\":\"remove\",\"by\":\"r9\",\"target\":\"id:a\"}]",
                &asked
            )
            .is_err()
        );
        assert!(ops_from_answer("no list here", &asked).is_err());
        let prompt = apply_prompt(
            "pages/home.html",
            "<main></main>",
            "- Campagnes",
            "- Menu (Navigation menu): a bar",
            &asked,
        );
        assert!(prompt.contains("request `r1` (on component:Campagnes): add an archive button"));
        assert!(prompt.contains("The kit's components:\n- Menu (Navigation menu): a bar"));
        assert!(prompt.contains(r#""kind":"mf""#));
        assert!(prompt.contains("`what: component`"));
        assert!(prompt.contains("missing component: <Name>"));
        assert!(
            checked_ops(&[
                json!({"kind": "props", "by": "human", "target": "data:k1", "props": {"n": 3}})
            ])
            .is_ok()
        );
        assert!(prompt.contains("<page>\n<main></main>\n</page>"));
    }

    #[test]
    fn a_fingerprint_is_the_build_s_digest_of_bytes() {
        assert_eq!(fingerprint(b""), "cbf29ce484222325");
        assert_ne!(fingerprint(b"a"), fingerprint(b"b"));
    }
}
