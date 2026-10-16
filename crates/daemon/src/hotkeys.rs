//! Global hotkeys (hybrid):
//! - macOS, Windows, Linux/X11: registered directly (keyboard and mouse buttons)
//! - Linux/Wayland: XDG GlobalShortcuts portal where available; otherwise bind
//!   keys in the compositor to CLI commands (`aw-manual popover`, …)

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use aw_manual_core::config::Config;
use aw_manual_core::model::Source;
use serde_json::json;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::app::{HotkeyErrors, Svc};
use crate::{mouse, popover};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    Popover,
    Stop,
    Resume,
    Favorite(String),
}

impl Action {
    fn id(&self) -> String {
        match self {
            Action::Popover => "popover".into(),
            Action::Stop => "stop".into(),
            Action::Resume => "resume".into(),
            Action::Favorite(id) => format!("favorite:{id}"),
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn description(&self) -> String {
        match self {
            Action::Popover => "Open the aw-manual popover".into(),
            Action::Stop => "Stop the running activity".into(),
            Action::Resume => "Resume the last stopped activity".into(),
            Action::Favorite(id) => format!("Start {id}"),
        }
    }
}

pub fn bindings(cfg: &Config) -> Vec<(String, Action)> {
    let mut out = Vec::new();
    if let Some(h) = &cfg.hotkeys.popover {
        out.push((h.clone(), Action::Popover));
    }
    if let Some(h) = &cfg.hotkeys.stop {
        out.push((h.clone(), Action::Stop));
    }
    if let Some(h) = &cfg.hotkeys.resume {
        out.push((h.clone(), Action::Resume));
    }
    for (id, h) in &cfg.hotkeys.favorites {
        out.push((h.clone(), Action::Favorite(id.clone())));
    }
    out
}

pub fn trigger(app: &AppHandle, action: &Action) {
    let svc = app.state::<Arc<Svc>>().inner().clone();
    let result = match action {
        Action::Popover => popover::open_or_confirm(app, Source::Hotkey)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        Action::Stop => svc
            .call("stop", json!({"source": Source::Hotkey}))
            .map(|_| ())
            .map_err(|e| e.message),
        Action::Resume => svc
            .call("resume", json!({"source": Source::Hotkey}))
            .map(|_| ())
            .map_err(|e| e.message),
        Action::Favorite(id) => popover::start_or_ask(app, id, Source::Hotkey, "hotkey"),
    };
    if let Err(e) = result {
        tracing::info!("hotkey action {} failed: {e}", action.id());
    }
}

/// Registers all hotkeys of the configuration (replacing earlier ones).
pub fn apply(app: &AppHandle, cfg: &Config) {
    let mut errors = Vec::new();
    let all = bindings(cfg);
    let (mouse_bindings, key_bindings): (Vec<_>, Vec<_>) = all
        .into_iter()
        .partition(|(h, _)| mouse::parse_button(h).is_some());

    // Keyboard.
    if popover::is_wayland() {
        #[cfg(target_os = "linux")]
        portal::apply(app.clone(), key_bindings);
    } else {
        let gs = app.global_shortcut();
        if let Err(e) = gs.unregister_all() {
            tracing::warn!("cannot unregister hotkeys: {e}");
        }
        for (hotkey, action) in key_bindings {
            let a = action.clone();
            let r = gs.on_shortcut(hotkey.as_str(), move |app, _shortcut, event| {
                tracing::debug!("hotkey {} {:?}", a.id(), event.state);
                if event.state == ShortcutState::Pressed {
                    trigger(app, &a);
                }
            });
            if let Err(e) = r {
                tracing::warn!("cannot register hotkey {hotkey}: {e}");
                errors.push(format!("{hotkey} ({}): {e}", action.id()));
            }
        }
    }

    // Mouse buttons.
    let mut buttons: HashMap<u8, Action> = HashMap::new();
    for (hotkey, action) in mouse_bindings {
        if let Some(n) = mouse::parse_button(&hotkey) {
            buttons.insert(n, action);
        }
    }
    let actions: Arc<Mutex<HashMap<u8, Action>>> = app.state::<MouseActions>().0.clone();
    *actions.lock().unwrap() = buttons.clone();
    let app2 = app.clone();
    let set: HashSet<u8> = buttons.keys().copied().collect();
    if let Err(e) = mouse::set_buttons(set, move |n| {
        let action = actions.lock().unwrap().get(&n).cloned();
        if let Some(action) = action {
            let app = app2.clone();
            // Leave the hook thread quickly.
            tauri::async_runtime::spawn(async move { trigger(&app, &action) });
        }
    }) {
        tracing::warn!("{e}");
        errors.push(e);
    }

    *app.state::<HotkeyErrors>().0.lock().unwrap() = errors;
}

/// Actions bound to mouse buttons.
#[derive(Default)]
pub struct MouseActions(pub Arc<Mutex<HashMap<u8, Action>>>);

#[cfg(target_os = "linux")]
mod portal {
    //! XDG GlobalShortcuts portal (KDE Plasma, GNOME 48+, Hyprland with xdg-desktop-portal-hyprland, …).

    use super::*;
    use std::time::Duration;

    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use ashpd::desktop::Session;
    use futures_util::StreamExt;
    use tokio::sync::Notify;

    /// The registration task and the signal that stops it. Stopping (instead
    /// of aborting) lets it close its portal session: a leftover session that
    /// holds the same shortcuts can make the portal reject the next binding.
    static TASK: Mutex<Option<(tauri::async_runtime::JoinHandle<()>, Arc<Notify>)>> =
        Mutex::new(None);
    static REGISTERED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    /// Waits between registration attempts. The portal (or its backend) is
    /// often not ready yet when the daemon starts with the graphical session.
    const RETRY_DELAYS: [u64; 7] = [1, 2, 4, 8, 15, 30, 60];

    /// Converts `Ctrl+Alt+Space` to the portal's trigger format `CTRL+ALT+space`.
    pub fn portal_trigger(hotkey: &str) -> String {
        hotkey
            .split('+')
            .map(|p| {
                let p = p.trim();
                match p.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" | "commandorcontrol" | "commandorctrl" | "cmdorctrl"
                    | "cmdorcontrol" => "CTRL".into(),
                    "alt" | "option" => "ALT".into(),
                    "shift" => "SHIFT".into(),
                    "super" | "cmd" | "command" => "LOGO".into(),
                    "space" => "space".into(),
                    "enter" | "return" => "Return".into(),
                    "esc" | "escape" => "Escape".into(),
                    other if other.len() == 1 => other.to_string(),
                    _ => p.to_string(),
                }
            })
            .collect::<Vec<_>>()
            .join("+")
    }

    pub fn apply(app: AppHandle, bindings: Vec<(String, Action)>) {
        let previous = TASK.lock().unwrap().take();
        let stop = Arc::new(Notify::new());
        let stop2 = stop.clone();
        let task = tauri::async_runtime::spawn(async move {
            if let Some((handle, stop)) = previous {
                stop.notify_one();
                let _ = handle.await;
            }
            if !bindings.is_empty() {
                register(&app, &bindings, &stop2).await;
            }
        });
        *TASK.lock().unwrap() = Some((task, stop));
    }

    /// Registers the shortcuts, retrying while the portal fails, and binds
    /// them again when the portal ends the session.
    async fn register(app: &AppHandle, bindings: &[(String, Action)], stop: &Notify) {
        let mut delays = RETRY_DELAYS.iter();
        loop {
            let mut bound = false;
            let reason = match run(app, bindings, stop, &mut bound).await {
                Ok(Ended::Stopped) => return,
                Ok(Ended::Closed) => "the portal closed the session".to_string(),
                Err(e) => e.to_string(),
            };
            if bound {
                delays = RETRY_DELAYS.iter();
            }
            let Some(&secs) = delays.next() else {
                let msg = format!(
                    "global shortcuts portal unavailable ({reason}); bind keys in your compositor to `aw-manual popover`, `aw-manual stop`, …"
                );
                tracing::warn!("{msg}");
                app.state::<HotkeyErrors>().0.lock().unwrap().push(msg);
                return;
            };
            tracing::info!(
                "cannot register hotkeys via the global shortcuts portal ({reason}), retrying in {secs}s"
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
                _ = stop.notified() => return,
            }
        }
    }

    enum Ended {
        /// A newer configuration replaces the shortcuts.
        Stopped,
        /// The portal ended the session (or its signals).
        Closed,
    }

    /// Unsandboxed apps have no app ID the portal can derive, and some backends
    /// (xdg-desktop-portal-gnome) refuse global shortcuts without one. Registers
    /// the bundle identifier (needs a matching `<identifier>.desktop` file) once
    /// per D-Bus connection, before any other portal call.
    async fn register_host_app(app: &AppHandle) {
        REGISTERED
            .get_or_init(|| async {
                let id = app.config().identifier.clone();
                let r = match ashpd::AppID::try_from(id.as_str()) {
                    Ok(app_id) => ashpd::register_host_app(app_id).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = r {
                    tracing::warn!("cannot register app id {id} with the portal: {e}");
                }
            })
            .await;
    }

    /// Binds the shortcuts in a new portal session and handles their
    /// activations until stopped or the session ends; then closes the session.
    async fn run(
        app: &AppHandle,
        bindings: &[(String, Action)],
        stop: &Notify,
        bound: &mut bool,
    ) -> ashpd::Result<Ended> {
        register_host_app(app).await;
        let portal = GlobalShortcuts::new().await?;
        let session = portal.create_session().await?;
        let result = serve(app, bindings, stop, bound, &portal, &session).await;
        if let Err(e) = session.close().await {
            tracing::debug!("cannot close the global shortcuts portal session: {e}");
        }
        result
    }

    async fn serve(
        app: &AppHandle,
        bindings: &[(String, Action)],
        stop: &Notify,
        bound: &mut bool,
        portal: &GlobalShortcuts<'_>,
        session: &Session<'_, GlobalShortcuts<'_>>,
    ) -> ashpd::Result<Ended> {
        let mut closed = session.receive_closed().await?;
        let shortcuts: Vec<NewShortcut> = bindings
            .iter()
            .map(|(h, a)| {
                NewShortcut::new(a.id(), a.description())
                    .preferred_trigger(Some(portal_trigger(h).as_str()))
            })
            .collect();
        tokio::select! {
            r = portal.bind_shortcuts(session, &shortcuts, None) => r?.response()?,
            _ = stop.notified() => return Ok(Ended::Stopped),
        };
        *bound = true;
        tracing::info!("hotkeys registered via the global shortcuts portal");
        let actions: HashMap<String, Action> =
            bindings.iter().map(|(_, a)| (a.id(), a.clone())).collect();
        let mut activated = portal.receive_activated().await?;
        loop {
            tokio::select! {
                event = activated.next() => {
                    let Some(event) = event else { return Ok(Ended::Closed) };
                    if let Some(action) = actions.get(event.shortcut_id()) {
                        trigger(app, action);
                    }
                }
                _ = closed.next() => return Ok(Ended::Closed),
                _ = stop.notified() => return Ok(Ended::Stopped),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn triggers() {
            assert_eq!(super::portal_trigger("Ctrl+Alt+Space"), "CTRL+ALT+space");
            assert_eq!(super::portal_trigger("Super+Shift+T"), "LOGO+SHIFT+t");
            assert_eq!(super::portal_trigger("Ctrl+F5"), "CTRL+F5");
        }
    }
}
