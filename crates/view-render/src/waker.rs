//! The event loop's proxy, kept so the page can wake a scene that waits.
//!
//! A scene at a dim pace, or paused, sleeps until its timer or an event,
//! and a pace or a picture pushed from the page is an event only once it is
//! sent through the proxy.
//!
//! Kept in a thread-local: the web runs one thread, and winit's proxy there
//! is not shared across threads. Natively nothing is kept — there is no
//! page to wake it.

use bevy::prelude::*;
use bevy::winit::EventLoopProxyWrapper;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PROXY: std::cell::RefCell<Option<bevy::winit::EventLoopProxy<bevy::winit::WinitUserEvent>>> =
        const { std::cell::RefCell::new(None) };
}

/// Keeps the proxy, once, at start.
// Natively the body is empty, and the lint asks for a `const fn` a system
// cannot be; on the web it does its work.
#[allow(clippy::needless_pass_by_value, clippy::missing_const_for_fn)]
pub fn keep(proxy: Option<Res<'_, EventLoopProxyWrapper>>) {
    #[cfg(target_arch = "wasm32")]
    if let Some(proxy) = proxy {
        let kept = (**proxy).clone();
        PROXY.with(|slot| *slot.borrow_mut() = Some(kept));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = proxy;
}

/// Wakes the scene, if it waits: its next frame reads what the page pushed.
// Natively there is nothing to wake, and the lint would make it `const`.
#[allow(clippy::missing_const_for_fn)]
pub fn wake() {
    #[cfg(target_arch = "wasm32")]
    PROXY.with(|slot| {
        if let Some(proxy) = slot.borrow().as_ref() {
            let _ = proxy.send_event(bevy::winit::WinitUserEvent::WakeUp);
        }
    });
}
