//! What crosses between the page and the scene, in both directions.
//!
//! Inbound: the page pushes the picture and the viewpoint; the scene reads
//! them at the start of a frame. Outbound: the scene reports pointer events
//! as `(kind, payload)` pairs the page already knows how to act on — the
//! same hotspot shape the canvas renderer used, so `app.js` keeps its panes.
//!
//! A `Mutex` rather than a channel: the page may push three pictures between
//! two frames, and only the last one matters.

use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

/// Where the viewer stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "UPPERCASE")]
pub enum View {
    /// Outside the plant.
    #[default]
    A,
    /// Inside, the six rooms.
    B,
    /// One room.
    C {
        /// The room's key: `lines`, `office`, `store`, `value`, `control`,
        /// `construction`.
        room: String,
    },
}

/// What a hotspot does when clicked — exactly what `app.js`'s `act()` reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Hot {
    /// Navigate to this level.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "go")]
    pub go: Option<String>,
    /// … into this room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    /// Open this pane (the page's own JSON shape, carried opaque).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<serde_json::Value>,
    /// Open this URL in a new tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The tooltip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<String>,
}

/// What the page last pushed.
#[derive(Debug, Default)]
pub struct Inbox {
    /// The picture, as JSON, when it changed since the last take.
    pub snapshot: Option<String>,
    /// The viewpoint, when it changed since the last take.
    pub view: Option<View>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox {
    snapshot: None,
    view: None,
});

fn inbox() -> MutexGuard<'static, Inbox> {
    INBOX.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The page pushed a picture.
pub fn push_snapshot(json: &str) {
    inbox().snapshot = Some(json.to_string());
}

/// The page moved the viewer.
pub fn push_view(view: View) {
    inbox().view = Some(view);
}

/// Everything pushed since the last take, and the inbox emptied.
#[must_use]
pub fn take() -> Inbox {
    std::mem::take(&mut *inbox())
}

/// What the scene tells the page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outbound {
    /// The pointer clicked a hotspot.
    Click {
        /// What to do.
        hot: Hot,
    },
    /// The pointer entered a hotspot.
    Hover {
        /// Its tooltip, when it has one.
        tip: Option<String>,
        /// Where the pointer is, in CSS pixels of the canvas.
        x: f32,
        /// Where the pointer is, in CSS pixels of the canvas.
        y: f32,
    },
    /// The pointer left every hotspot.
    Leave,
    /// The scene is up and drew its first frame.
    Ready,
}

/// Tells the page. In the browser this reaches `window.harnessRender.onEvent`;
/// in a native build (tests, the workspace's own `cargo build`) it is a log
/// line, since there is no page.
pub fn emit(out: &Outbound) {
    let Ok(json) = serde_json::to_string(out) else {
        return;
    };
    #[cfg(target_arch = "wasm32")]
    crate::web::on_event(&json);
    #[cfg(not(target_arch = "wasm32"))]
    bevy::log::debug!("render event: {json}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbox_keeps_only_the_last_push_and_empties_on_take() {
        push_snapshot("{\"a\":1}");
        push_snapshot("{\"a\":2}");
        push_view(View::C {
            room: "lines".to_string(),
        });
        let taken = take();
        assert_eq!(taken.snapshot.as_deref(), Some("{\"a\":2}"));
        assert_eq!(
            taken.view,
            Some(View::C {
                room: "lines".to_string()
            })
        );
        let again = take();
        assert!(again.snapshot.is_none() && again.view.is_none());
    }

    #[test]
    fn a_view_reads_from_the_page_s_json() {
        let c: View = serde_json::from_str(r#"{"level":"C","room":"store"}"#).expect("view");
        assert_eq!(
            c,
            View::C {
                room: "store".to_string()
            }
        );
        let a: View = serde_json::from_str(r#"{"level":"A"}"#).expect("view");
        assert_eq!(a, View::A);
    }

    #[test]
    fn an_outbound_click_carries_the_hotspot_the_page_acts_on() {
        let hot = Hot {
            pane: Some(serde_json::json!({"kind": "board"})),
            tip: Some("the sign".to_string()),
            ..Hot::default()
        };
        let json = serde_json::to_string(&Outbound::Click { hot }).expect("json");
        assert_eq!(
            json,
            r#"{"kind":"click","hot":{"pane":{"kind":"board"},"tip":"the sign"}}"#
        );
    }
}
