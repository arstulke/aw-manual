//! Tray / status bar icon with its menu and the configurable text next to it.

use std::sync::{Arc, Mutex};

use aw_manual_core::config::TrayConfig;
use aw_manual_core::display;
use aw_manual_core::engine::RunningInfo;
use aw_manual_core::engine::Status;
use aw_manual_core::favorites::Favorite;
use aw_manual_core::format::{format_clock, format_elapsed, PATH_SEPARATOR};
use aw_manual_core::model::Source;
use chrono::{DateTime, Local, Utc};
use serde_json::json;
use tauri::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::app::{open_settings, Svc};
use crate::icons::{self, IconState};
use crate::popover;

const TRAY_ID: &str = "main";

#[derive(Default)]
struct Cache {
    menu_key: Mutex<String>,
    icon_key: Mutex<Option<(IconState, bool)>>,
    text: Mutex<Option<Option<String>>>,
    tooltip: Mutex<Option<String>>,
    /// The menu line with the elapsed time of the running activity, updated
    /// in place (the tray text next to the icon may be clipped).
    elapsed_item: Mutex<Option<MenuItem<Wry>>>,
    elapsed_text: Mutex<Option<String>>,
}

fn status(svc: &Svc) -> Status {
    serde_json::from_value(svc.status_json()).expect("status deserializes")
}

pub fn create(app: &AppHandle, svc: &Arc<Svc>) -> tauri::Result<()> {
    app.manage(Cache::default());
    let menu = Menu::new(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icons::image(&IconState::Idle, false))
        .tooltip("aw-manual")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let _ = if popover::is_visible(app) {
                    popover::hide(app)
                } else {
                    popover::show(app, "tray")
                };
            }
        })
        .build(app)?;
    update(app, svc);
    Ok(())
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref().to_string();
    let svc = app.state::<Arc<Svc>>().inner().clone();
    let result = match id.as_str() {
        "open" => popover::show(app, "tray").map_err(|e| e.to_string()),
        "stop" => svc
            .call("stop", json!({"source": "tray"}))
            .map(|_| ())
            .map_err(|e| e.message),
        "resume" => svc
            .call("resume", json!({"source": "tray"}))
            .map(|_| ())
            .map_err(|e| e.message),
        "note" => popover::show_with(
            app,
            popover::Opened {
                note: true,
                ..popover::Opened::new("note")
            },
        )
        .map_err(|e| e.to_string()),
        "settings" => open_settings(app).map_err(|e| e.to_string()),
        "quit" => {
            app.exit(0);
            Ok(())
        }
        other => match other.strip_prefix("fav:") {
            Some(fav) => popover::start_or_ask(app, fav, Source::Tray, "tray"),
            None => Ok(()),
        },
    };
    if let Err(e) = result {
        tracing::warn!("tray action \"{id}\" failed: {e}");
    }
}

fn favorites_menu(
    app: &AppHandle,
    nodes: &[Favorite],
    running: Option<&str>,
) -> tauri::Result<Vec<Box<dyn IsMenuItem<Wry>>>> {
    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    for n in nodes {
        if n.is_leaf() {
            let mut label = if running == Some(n.id.as_str()) {
                format!("● {}", n.title)
            } else {
                n.title.clone()
            };
            if n.ask_note {
                label.push('…');
            }
            items.push(Box::new(MenuItem::with_id(
                app,
                format!("fav:{}", n.id),
                label,
                true,
                None::<&str>,
            )?));
        } else {
            let sub = Submenu::with_id(app, format!("folder:{}", n.id), &n.title, true)?;
            for child in favorites_menu(app, &n.children, running)? {
                sub.append(child.as_ref())?;
            }
            items.push(Box::new(sub));
        }
    }
    Ok(items)
}

fn menu_key(st: &Status, svc: &Svc) -> String {
    let fav = serde_json::to_string(
        &svc.loaded()
            .favorites
            .tree
            .roots
            .iter()
            .map(|f| (&f.id, &f.title))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    format!(
        "{:?}|{:?}|{:?}|{:?}|{}|{}|{fav}",
        st.running.as_ref().map(|r| (&r.id, &r.note)),
        st.last_stopped.as_ref().map(|s| (&s.id, &s.note)),
        st.server.online,
        st.server.pending,
        st.server.failed,
        svc.loaded().favorites.tree.all().len(),
    )
}

/// "Running for 1h 23m (since 09:30)".
fn elapsed_line(cfg: &TrayConfig, r: &RunningInfo, now: DateTime<Utc>) -> String {
    format!(
        "Running for {} (since {})",
        format_elapsed((now - r.start).num_seconds(), cfg.elapsed_format),
        format_clock(&r.start.with_timezone(&Local), cfg.time_format)
    )
}

fn build_menu(
    app: &AppHandle,
    svc: &Svc,
    st: &Status,
) -> tauri::Result<(Menu<Wry>, Option<MenuItem<Wry>>)> {
    let menu = Menu::new(app)?;
    let header = match &st.running {
        Some(r) => format!("● {}", r.path.join(PATH_SEPARATOR)),
        None => "No activity running".to_string(),
    };
    menu.append(&MenuItem::with_id(
        app,
        "info",
        header,
        false,
        None::<&str>,
    )?)?;
    let elapsed_item = match &st.running {
        Some(r) => {
            let item = MenuItem::with_id(
                app,
                "info-elapsed",
                elapsed_line(&svc.config().tray, r, Utc::now()),
                false,
                None::<&str>,
            )?;
            menu.append(&item)?;
            Some(item)
        }
        None => None,
    };
    if st.server.online == Some(false) || st.server.pending > 0 {
        let text = if st.server.online == Some(false) {
            format!("Offline – {} entries pending", st.server.pending)
        } else {
            format!("{} entries pending", st.server.pending)
        };
        menu.append(&MenuItem::with_id(
            app,
            "info-server",
            text,
            false,
            None::<&str>,
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "open",
        "Open popover",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "stop",
        "Stop",
        st.running.is_some(),
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "note",
        "Leave a note…",
        st.running.is_some(),
        None::<&str>,
    )?)?;
    let resume_label = match &st.last_stopped {
        Some(s) => format!("Resume {}", s.path.join(PATH_SEPARATOR)),
        None => "Resume".to_string(),
    };
    let can_resume = st.last_stopped.as_ref().is_some_and(|s| {
        st.running
            .as_ref()
            .is_none_or(|r| r.id != s.id || r.note != s.note)
    });
    menu.append(&MenuItem::with_id(
        app,
        "resume",
        resume_label,
        can_resume,
        None::<&str>,
    )?)?;
    let favorites = Submenu::with_id(app, "favorites", "Favorites", true)?;
    let loaded = svc.loaded();
    let items = favorites_menu(
        app,
        &loaded.favorites.tree.roots,
        st.running.as_ref().map(|r| r.id.as_str()),
    )?;
    drop(loaded);
    if items.is_empty() {
        favorites.append(&MenuItem::with_id(
            app,
            "no-favorites",
            "No favorites defined",
            false,
            None::<&str>,
        )?)?;
    }
    for item in &items {
        favorites.append(item.as_ref())?;
    }
    menu.append(&favorites)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "settings",
        "Settings…",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "Quit aw-manual",
        true,
        None::<&str>,
    )?)?;
    Ok((menu, elapsed_item))
}

/// Updates icon, menu, text and tooltip after a change.
pub fn update(app: &AppHandle, svc: &Arc<Svc>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let cache = app.state::<Cache>();
    let st = status(svc);
    let cfg = svc.config();

    let offline = st.server.online == Some(false) || st.server.failed > 0;
    let icon_state = match &st.running {
        None => IconState::Idle,
        Some(r) => IconState::Active(if cfg.tray.color_icon {
            r.color.clone()
        } else {
            None
        }),
    };
    let key = Some((icon_state.clone(), offline));
    if *cache.icon_key.lock().unwrap() != key {
        if let Err(e) = tray.set_icon(Some(icons::image(&icon_state, offline))) {
            tracing::warn!("cannot set the tray icon: {e}");
        }
        *cache.icon_key.lock().unwrap() = key;
    }

    let mk = menu_key(&st, svc);
    if *cache.menu_key.lock().unwrap() != mk {
        match build_menu(app, svc, &st) {
            Ok((menu, elapsed_item)) => {
                if let Err(e) = tray.set_menu(Some(menu)) {
                    tracing::warn!("cannot set the tray menu: {e}");
                }
                *cache.menu_key.lock().unwrap() = mk;
                *cache.elapsed_text.lock().unwrap() = None;
                *cache.elapsed_item.lock().unwrap() = elapsed_item;
            }
            Err(e) => tracing::warn!("cannot build the tray menu: {e}"),
        }
    }
    update_text(app, svc);
}

/// Updates the text next to the icon and the tooltip (called every second).
pub fn update_text(app: &AppHandle, svc: &Arc<Svc>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let cache = app.state::<Cache>();
    let cfg = svc.config();
    let now = Utc::now();
    let st = status(svc);
    let (text, tooltip) = match &st.running {
        Some(r) => (
            display::tray_text(&cfg.tray, r, now),
            display::tooltip(&cfg.tray, r, now),
        ),
        None => (None, "aw-manual – no activity running".to_string()),
    };
    let tooltip = if st.server.online == Some(false) {
        format!(
            "{tooltip}\naw-server unreachable, {} entries pending",
            st.server.pending
        )
    } else {
        tooltip
    };
    let mut cached_text = cache.text.lock().unwrap();
    if cached_text.as_ref() != Some(&text) {
        // Text next to the icon: macOS menu bar, Linux AppIndicator label; not supported on Windows.
        let _ = tray.set_title(text.as_deref());
        *cached_text = Some(text);
    }
    if let (Some(item), Some(r)) = (cache.elapsed_item.lock().unwrap().as_ref(), &st.running) {
        let line = elapsed_line(&cfg.tray, r, now);
        let mut cached = cache.elapsed_text.lock().unwrap();
        if cached.as_ref() != Some(&line) {
            let _ = item.set_text(&line);
            *cached = Some(line);
        }
    }
    let mut cached_tip = cache.tooltip.lock().unwrap();
    if cached_tip.as_ref() != Some(&tooltip) {
        let _ = tray.set_tooltip(Some(&tooltip));
        *cached_tip = Some(tooltip);
    }
}
