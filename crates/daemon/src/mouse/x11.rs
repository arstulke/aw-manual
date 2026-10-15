//! X11 backend: passive grabs of the configured buttons on the root window.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ButtonIndex, ConnectionExt, EventMask, GrabMode, ModMask};
use x11rb::protocol::Event;

use super::Shared;

/// `MouseN` → X11 button number (Mouse4 = back = 8, Mouse5 = forward = 9).
fn x11_button(n: u8) -> u8 {
    n + 4
}

pub fn start(shared: Arc<Shared>) -> Result<(), String> {
    let (conn, screen) =
        x11rb::connect(None).map_err(|e| format!("cannot connect to the X server: {e}"))?;
    let root = conn.setup().roots[screen].root;
    std::thread::Builder::new()
        .name("aw-manual-mouse".into())
        .spawn(move || {
            let mut grabbed: HashSet<u8> = HashSet::new();
            let mut generation = u64::MAX;
            loop {
                if shared.generation() != generation {
                    generation = shared.generation();
                    let wanted = shared.buttons();
                    for b in grabbed.difference(&wanted) {
                        let _ = conn.ungrab_button(
                            ButtonIndex::from(x11_button(*b)),
                            root,
                            ModMask::ANY,
                        );
                    }
                    for b in wanted.difference(&grabbed) {
                        let r = conn
                            .grab_button(
                                false,
                                root,
                                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                                GrabMode::ASYNC,
                                GrabMode::ASYNC,
                                x11rb::NONE,
                                x11rb::NONE,
                                ButtonIndex::from(x11_button(*b)),
                                ModMask::ANY,
                            )
                            .map(|c| c.check());
                        match r {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => tracing::warn!(
                                "cannot grab Mouse{b} (is it used by another program?): {e:?}"
                            ),
                            Err(e) => tracing::warn!("cannot grab Mouse{b}: {e}"),
                        }
                    }
                    grabbed = wanted;
                    let _ = conn.flush();
                }
                match conn.poll_for_event() {
                    Ok(Some(Event::ButtonPress(e))) => {
                        let n = e.detail.saturating_sub(4);
                        if shared.wants(n) {
                            shared.pressed(n);
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => std::thread::sleep(Duration::from_millis(15)),
                    Err(e) => {
                        tracing::error!("X11 connection lost, mouse hotkeys stop working: {e}");
                        return;
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
