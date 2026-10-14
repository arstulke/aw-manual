//! The popover window. It is created once at startup and kept alive (hidden),
//! so it appears without delay.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use aw_manual_core::ipc::NavCommand;
use aw_manual_core::model::Source;
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

use crate::app::Svc;

pub const LABEL: &str = "popover";
const WIDTH: f64 = 500.0;
const HEIGHT: f64 = 460.0;

#[derive(Default)]
struct State {
    visible: AtomicBool,
    shown_at: Mutex<Option<Instant>>,
}

/// Payload of the `popover:opened` event.
#[derive(Clone, Default, Serialize)]
pub struct Opened {
    /// What opened the popover: "hotkey", "tray", "cli", "launch", "note", "afk".
    pub trigger: String,
    /// Ask for a note for this favorite, then start it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorite: Option<String>,
    /// Open the note field for the running activity.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub note: bool,
    /// Ask what the user did during this AFK period (retroactive entry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub afk: Option<aw_manual_core::afk::AfkPeriod>,
}

impl Opened {
    pub fn new(trigger: &str) -> Self {
        Opened {
            trigger: trigger.to_string(),
            ..Default::default()
        }
    }
}

/// True on a native Wayland session, where windows cannot position themselves.
pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland"))
        && std::env::var("GDK_BACKEND").map_or(true, |b| !b.starts_with("x11"))
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    app.manage(State::default());
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("popover.html".into()))
        .title("aw-manual")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .center()
        .build()?;
    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            // Close when the user clicks elsewhere. Keyboard grabs (e.g. by the
            // global hotkey) cause short focus-out/in flickers, so check again
            // after a moment whether the focus is really gone.
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let state = handle.state::<State>();
                let recent = state
                    .shown_at
                    .lock()
                    .unwrap()
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(500));
                let focused = self::window(&handle)
                    .and_then(|w| w.is_focused())
                    .unwrap_or(true);
                if state.visible.load(Ordering::SeqCst) && !recent && !focused {
                    tracing::debug!("popover lost the focus, hiding it");
                    let _ = hide(&handle);
                }
            });
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = hide(&handle);
        }
        _ => {}
    });
    Ok(())
}

fn window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    app.get_webview_window(LABEL)
        .ok_or(tauri::Error::WindowNotFound)
}

pub fn is_visible(app: &AppHandle) -> bool {
    app.state::<State>().visible.load(Ordering::SeqCst)
}

/// Places the window under the mouse pointer, inside the monitor's work area.
fn position_at_cursor(app: &AppHandle, w: &WebviewWindow) -> tauri::Result<()> {
    let cursor = app.cursor_position()?;
    let Some(monitor) = app.monitor_from_point(cursor.x, cursor.y)? else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let (ww, wh) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32);
    let area = monitor.work_area();
    let (ax, ay) = (area.position.x, area.position.y);
    let (aw, ah) = (area.size.width as i32, area.size.height as i32);
    // The pointer ends up over the upper part of the list.
    let mut x = cursor.x as i32 - ww / 2;
    let mut y = cursor.y as i32 - (110.0 * scale) as i32;
    x = x.clamp(ax, (ax + aw - ww).max(ax));
    y = y.clamp(ay, (ay + ah - wh).max(ay));
    w.set_position(PhysicalPosition::new(x, y))
}

pub fn show(app: &AppHandle, trigger: &str) -> tauri::Result<()> {
    show_with(app, Opened::new(trigger))
}

pub fn show_with(app: &AppHandle, opened: Opened) -> tauri::Result<()> {
    let w = window(app)?;
    let open_at_cursor = app
        .state::<std::sync::Arc<Svc>>()
        .config()
        .popover
        .scroll
        .open_at_cursor;
    if open_at_cursor && !is_wayland() {
        if let Err(e) = position_at_cursor(app, &w) {
            tracing::debug!("cannot position the popover at the cursor: {e}");
        }
    } else if !is_wayland() {
        let _ = w.center();
    }
    let state = app.state::<State>();
    *state.shown_at.lock().unwrap() = Some(Instant::now());
    state.visible.store(true, Ordering::SeqCst);
    app.emit("popover:opened", opened)?;
    w.show()?;
    w.set_focus()?;
    Ok(())
}

pub fn hide(app: &AppHandle) -> tauri::Result<()> {
    tracing::debug!("hiding the popover");
    let state = app.state::<State>();
    state.visible.store(false, Ordering::SeqCst);
    let w = window(app)?;
    w.hide()?;
    let _ = app.emit("popover:closed", ());
    Ok(())
}

/// The popover hotkey: closed → open; open → confirm (scroll mode).
pub fn open_or_confirm(app: &AppHandle, source: Source) -> tauri::Result<&'static str> {
    tracing::debug!(
        "popover command from {source:?}, visible: {}",
        is_visible(app)
    );
    if is_visible(app) {
        let confirm = app
            .state::<std::sync::Arc<Svc>>()
            .config()
            .popover
            .scroll
            .confirm_with_hotkey;
        if confirm {
            app.emit("popover:nav", NavCommand::Confirm)?;
            Ok("confirmed")
        } else {
            hide(app)?;
            Ok("closed")
        }
    } else {
        let trigger = match source {
            Source::Hotkey => "hotkey",
            Source::Tray => "tray",
            _ => "cli",
        };
        show(app, trigger)?;
        Ok("opened")
    }
}

/// Forwards a navigation command to the open popover.
pub fn nav(app: &AppHandle, command: NavCommand) -> tauri::Result<bool> {
    if !is_visible(app) {
        return Ok(false);
    }
    app.emit("popover:nav", command)?;
    Ok(true)
}

/// Starts a favorite from the tray or a hotkey. Favorites with `ask_note`
/// open the popover with the note field instead.
pub fn start_or_ask(
    app: &AppHandle,
    id: &str,
    source: Source,
    trigger: &str,
) -> Result<(), String> {
    let svc = app.state::<std::sync::Arc<Svc>>().inner().clone();
    let ask = svc
        .loaded()
        .favorites
        .tree
        .find(id)
        .is_some_and(|f| f.is_leaf() && f.ask_note);
    if ask {
        return show_with(
            app,
            Opened {
                favorite: Some(id.to_string()),
                ..Opened::new(trigger)
            },
        )
        .map_err(|e| e.to_string());
    }
    svc.call(
        "start",
        serde_json::json!({"target": {"favorite": id}, "source": source}),
    )
    .map(|_| ())
    .map_err(|e| e.message)
}
