//! What the browser calls, and what it is called back with.
//!
//! Three exports — start, push a picture, move the viewer — and one callback:
//! `window.harnessRender.onEvent(json)`, looked up at call time through
//! `js_sys::Reflect` rather than declared as an import, so that no `extern`
//! block and no `unsafe` enter the crate.

use wasm_bindgen::prelude::*;

use crate::bridge::{self, View};

/// Starts the scene in the canvas named by `selector`, e.g. `#scene`.
#[wasm_bindgen]
pub fn start(selector: &str) {
    console_error_panic_hook::set_once();
    crate::app::run(selector);
}

/// The page pushes the picture it received, as JSON.
#[wasm_bindgen(js_name = setSnapshot)]
pub fn set_snapshot(json: &str) {
    bridge::push_snapshot(json);
}

/// The page moves the viewer: `{"level":"A"}`, `{"level":"C","room":"lines"}`.
#[wasm_bindgen(js_name = setView)]
pub fn set_view(json: &str) {
    match serde_json::from_str::<View>(json) {
        Ok(view) => bridge::push_view(view),
        Err(e) => web_log(&format!("harness-render: not a view: {json} ({e})")),
    }
}

fn web_log(text: &str) {
    web_sys::console::warn_1(&JsValue::from_str(text));
}

/// Calls `window.harnessRender.onEvent(json)`, if the page defined it.
pub fn on_event(json: &str) {
    let global = js_sys::global();
    let Ok(bridge_obj) = js_sys::Reflect::get(&global, &JsValue::from_str("harnessRender")) else {
        return;
    };
    let Ok(handler) = js_sys::Reflect::get(&bridge_obj, &JsValue::from_str("onEvent")) else {
        return;
    };
    if let Some(function) = handler.dyn_ref::<js_sys::Function>() {
        let _ = function.call1(&bridge_obj, &JsValue::from_str(json));
    }
}
