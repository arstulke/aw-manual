//! The popover window. It is created once at startup and kept alive (hidden),
//! so it appears without delay.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use aw_manual_core::ipc::NavCommand;
use aw_manual_core::model::Source;
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

use crate::app::Svc;

pub const LABEL: &str = "popover";
/// `Opened::trigger` of a popover opened by a hand above a device.
pub const HOVER_TRIGGER: &str = "hover";
const WIDTH: f64 = 500.0;
/// Tall enough for the questions after a restart or unlock plus several rows.
const HEIGHT: f64 = 580.0;

#[derive(Default)]
struct State {
    visible: AtomicBool,
    shown_at: Mutex<Option<Instant>>,
    /// The latest request, also for a page that loads after the event was sent.
    last_opened: Mutex<Option<Opened>>,
    seq: std::sync::atomic::AtomicU64,
    /// The visible popover was opened by a hovering hand and nothing was
    /// confirmed in it yet, so the hand leaving closes it again.
    opened_by_hover: AtomicBool,
}

/// Payload of the `popover:opened` event.
#[derive(Clone, Default, Serialize)]
pub struct Opened {
    /// Increases with every request (the page handles each one once).
    pub seq: u64,
    /// What opened the popover: "hotkey", "tray", "cli", "launch", "note",
    /// "restart", "unlock".
    pub trigger: String,
    /// Ask for a note for this favorite, then start it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorite: Option<String>,
    /// Open the note field for the running activity.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub note: bool,
    /// Ask what the user did during this gap, then what to continue with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<aw_manual_core::gap::Gap>,
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

pub fn show_with(app: &AppHandle, mut opened: Opened) -> tauri::Result<()> {
    let w = window(app)?;
    // Some window managers (e.g. GTK client-side decorations on Wayland) add
    // their frame to the size each time a hidden window is shown again.
    if let Err(e) = w.set_size(LogicalSize::new(WIDTH, HEIGHT)) {
        tracing::debug!("cannot reset the popover size: {e}");
    }
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
    state
        .opened_by_hover
        .store(opened.trigger == HOVER_TRIGGER, Ordering::SeqCst);
    opened.seq = state.seq.fetch_add(1, Ordering::SeqCst) + 1;
    *state.last_opened.lock().unwrap() = Some(opened.clone());
    app.emit("popover:opened", opened)?;
    w.show()?;
    w.set_focus()?;
    Ok(())
}

pub fn hide(app: &AppHandle) -> tauri::Result<()> {
    tracing::debug!("hiding the popover");
    let state = app.state::<State>();
    state.visible.store(false, Ordering::SeqCst);
    state.opened_by_hover.store(false, Ordering::SeqCst);
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
            keep_open(app);
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

/// Confirming may open a note field or a question; the popover then no longer
/// closes when the hand leaves.
fn keep_open(app: &AppHandle) {
    app.state::<State>()
        .opened_by_hover
        .store(false, Ordering::SeqCst);
}

/// Closes the popover if a hovering hand opened it and nothing was confirmed
/// in it since. Returns whether it was closed.
pub fn close_if_opened_by_hover(app: &AppHandle) -> tauri::Result<bool> {
    let hover = app
        .state::<State>()
        .opened_by_hover
        .swap(false, Ordering::SeqCst);
    if hover && is_visible(app) {
        hide(app)?;
        return Ok(true);
    }
    Ok(false)
}

/// Forwards a navigation command to the open popover.
pub fn nav(app: &AppHandle, command: NavCommand) -> tauri::Result<bool> {
    if !is_visible(app) {
        return Ok(false);
    }
    if command == NavCommand::Confirm {
        keep_open(app);
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

/// The latest open request (for the page when it finishes loading).
pub fn last_opened(app: &AppHandle) -> Option<Opened> {
    app.state::<State>().last_opened.lock().unwrap().clone()
}

/// Opens the popover and asks what the user did during the gap.
pub fn ask_gap(app: &AppHandle, gap: aw_manual_core::gap::Gap) -> tauri::Result<()> {
    let trigger = match gap.reason {
        aw_manual_core::gap::GapReason::Restart => "restart",
        aw_manual_core::gap::GapReason::Unlock => "unlock",
    };
    show_with(
        app,
        Opened {
            gap: Some(gap),
            ..Opened::new(trigger)
        },
    )
}
