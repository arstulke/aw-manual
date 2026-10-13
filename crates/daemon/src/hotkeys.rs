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
        Action::Favorite(id) => svc
            .call(
                "start",
                json!({"target": {"favorite": id}, "source": Source::Hotkey}),
            )
            .map(|_| ())
            .map_err(|e| e.message),
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
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;

    static TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);
    static REGISTERED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

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
        if let Some(t) = TASK.lock().unwrap().take() {
            t.abort();
        }
        if bindings.is_empty() {
            return;
        }
        let task = tauri::async_runtime::spawn(async move {
            if let Err(e) = run(&app, bindings).await {
                let msg = format!(
                    "global shortcuts portal unavailable ({e}); bind keys in your compositor to `aw-manual popover`, `aw-manual stop`, …"
                );
                tracing::warn!("{msg}");
                app.state::<HotkeyErrors>().0.lock().unwrap().push(msg);
            }
        });
        *TASK.lock().unwrap() = Some(task);
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

    async fn run(app: &AppHandle, bindings: Vec<(String, Action)>) -> ashpd::Result<()> {
        register_host_app(app).await;
        let portal = GlobalShortcuts::new().await?;
        let session = portal.create_session().await?;
        let shortcuts: Vec<NewShortcut> = bindings
            .iter()
            .map(|(h, a)| {
                NewShortcut::new(a.id(), a.description())
                    .preferred_trigger(Some(portal_trigger(h).as_str()))
            })
            .collect();
        portal
            .bind_shortcuts(&session, &shortcuts, None)
            .await?
            .response()?;
        tracing::info!("hotkeys registered via the global shortcuts portal");
        let actions: HashMap<String, Action> =
            bindings.into_iter().map(|(_, a)| (a.id(), a)).collect();
        let mut activated = portal.receive_activated().await?;
        while let Some(event) = activated.next().await {
            if let Some(action) = actions.get(event.shortcut_id()) {
                trigger(app, action);
            }
        }
        drop(session);
        Ok(())
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
