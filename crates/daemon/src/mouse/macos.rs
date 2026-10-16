//! macOS backend: a Quartz event tap for "other" mouse buttons.
//! Requires the Accessibility permission (System Settings → Privacy & Security).

use std::sync::Arc;

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};

use super::Shared;

/// macOS button number (0 = left, 1 = right, 2 = middle, 3 = back, 4 = forward) → `MouseN`.
fn button_of(number: i64) -> u8 {
    u8::try_from(number + 1).unwrap_or(0)
}

pub fn start(shared: Arc<Shared>) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("aw-manual-mouse".into())
        .spawn(move || {
            let callback_shared = shared.clone();
            let result = CGEventTap::with_enabled(
                CGEventTapLocation::HID,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                vec![CGEventType::OtherMouseDown, CGEventType::OtherMouseUp],
                move |_proxy, etype, event| {
                    let n = button_of(event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER));
                    if !callback_shared.wants(n) {
                        return CallbackResult::Keep;
                    }
                    if matches!(etype, CGEventType::OtherMouseDown) {
                        callback_shared.pressed(n);
                    }
                    CallbackResult::Drop
                },
                || {
                    let _ = tx.send(Ok(()));
                    CFRunLoop::run_current();
                },
            );
            if result.is_err() {
                let _ = tx.send(Err(
                    "cannot capture mouse buttons: allow aw-manual in System Settings → Privacy & Security → Accessibility"
                        .into(),
                ));
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv()
        .unwrap_or_else(|_| Err("the mouse hook thread ended unexpectedly".into()))
}
