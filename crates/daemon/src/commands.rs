//! Commands invoked by the popover and settings windows.

use std::sync::Arc;

use aw_manual_core::engine::Target;
use aw_manual_core::format::{format_clock, format_elapsed};
use aw_manual_core::model::Source;
use aw_manual_core::timeparse;
use aw_manual_core::yaml_edit::NewFavorite;
use chrono::Local;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::app::{HotkeyErrors, Svc};
use crate::settings::{self, Change};

type Svcs<'a> = State<'a, Arc<Svc>>;

fn popover_source(source: Option<String>) -> Source {
    match source.as_deref() {
        Some("popover-scroll") => Source::PopoverScroll,
        _ => Source::Popover,
    }
}

/// Everything the popover needs to render.
#[tauri::command]
pub fn get_snapshot(svc: Svcs<'_>) -> Value {
    let cfg = svc.config();
    let loaded = svc.loaded();
    json!({
        "status": svc.status_json(),
        "favorites": loaded.favorites.tree.roots,
        "scroll": cfg.popover.scroll,
        "format": cfg.format,
        "hotkeys": cfg.hotkeys,
    })
}

#[tauri::command]
pub fn start_favorite(
    svc: Svcs<'_>,
    id: String,
    note: Option<String>,
    source: Option<String>,
) -> Result<Value, String> {
    svc.call(
        "start",
        json!({ "target": Target::Favorite(id), "note": note, "source": popover_source(source) }),
    )
    .map_err(|e| e.message)
}

#[tauri::command]
pub fn stop(svc: Svcs<'_>, source: Option<String>) -> Result<Value, String> {
    svc.call("stop", json!({ "source": popover_source(source) }))
        .map_err(|e| e.message)
}

#[tauri::command]
pub fn resume(svc: Svcs<'_>) -> Result<Value, String> {
    svc.call("resume", json!({ "source": Source::Popover }))
        .map_err(|e| e.message)
}

/// The latest request that opened the popover (the page may load after it).
#[tauri::command]
pub fn last_opened(app: AppHandle) -> Value {
    serde_json::to_value(crate::popover::last_opened(&app)).unwrap_or(Value::Null)
}

#[tauri::command]
pub fn set_note(svc: Svcs<'_>, note: Option<String>) -> Result<Value, String> {
    svc.call("note", json!({ "note": note, "source": Source::Popover }))
        .map_err(|e| e.message)
}

#[tauri::command]
pub fn preview_range(svc: Svcs<'_>, range: String) -> Result<Value, String> {
    let (start, end) = timeparse::parse_range(&range, &Local::now())?;
    let cfg = svc.config().format;
    let text = format!(
        "{}–{} ({})",
        format_clock(&start.with_timezone(&Local), cfg.time),
        format_clock(&end.with_timezone(&Local), cfg.time),
        format_elapsed((end - start).num_seconds(), cfg.elapsed)
    );
    let day = start.with_timezone(&Local).date_naive();
    let text = if day != Local::now().date_naive() {
        format!("{} {text}", day.format("%Y-%m-%d"))
    } else {
        text
    };
    Ok(json!({ "start": start, "end": end, "text": text }))
}

#[tauri::command]
pub fn log_entry(
    svc: Svcs<'_>,
    id: String,
    range: String,
    note: Option<String>,
) -> Result<Value, String> {
    let (start, end) = timeparse::parse_range(&range, &Local::now())?;
    svc.call(
        "log",
        json!({ "target": Target::Favorite(id), "start": start, "end": end, "note": note, "source": Source::Popover }),
    )
    .map_err(|e| e.message)
}

#[tauri::command]
pub fn hide_popover(app: AppHandle) -> Result<(), String> {
    crate::popover::hide(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    crate::popover::hide(&app).map_err(|e| e.to_string())?;
    crate::app::open_settings(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_settings(app: AppHandle, svc: Svcs<'_>, errors: State<'_, HotkeyErrors>) -> Value {
    use tauri_plugin_autostart::ManagerExt;
    let mut list = svc.list_json();
    let config_path = svc.config_dir().join(aw_manual_core::config::CONFIG_FILE);
    let extra = json!({
        "config": svc.config(),
        "config_path": config_path,
        "favorite_dirs": svc.config().favorite_dirs(svc.config_dir()),
        "writable_files": settings::writable_files(&svc),
        "ui_file": svc.config().favorites.default_file,
        "hotkey_errors": *errors.0.lock().unwrap(),
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "wayland": crate::popover::is_wayland(),
        "os": std::env::consts::OS,
        "version": env!("CARGO_PKG_VERSION"),
        "log_dir": aw_manual_core::paths::log_dir(),
        "status": svc.status_json(),
    });
    if let (Some(a), Some(b)) = (list.as_object_mut(), extra.as_object()) {
        a.insert(
            "config_state".into(),
            a.get("config").cloned().unwrap_or(Value::Null),
        );
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
    list
}

#[tauri::command]
pub async fn save_settings(svc: Svcs<'_>, changes: Vec<Change>) -> Result<(), Vec<String>> {
    let svc = svc.inner().clone();
    settings::save_config(svc.config_dir(), &changes)?;
    svc.reload().await;
    Ok(())
}

#[tauri::command]
pub async fn add_favorite(
    svc: Svcs<'_>,
    parent: Option<String>,
    file: Option<String>,
    favorite: NewFavorite,
) -> Result<String, String> {
    let svc = svc.inner().clone();
    let file = file.filter(|f| !f.is_empty()).map(std::path::PathBuf::from);
    let id = settings::add_favorite(&svc, parent.as_deref(), file.as_deref(), &favorite)?;
    svc.reload().await;
    Ok(id)
}

#[tauri::command]
pub async fn remove_favorite(svc: Svcs<'_>, id: String) -> Result<(), String> {
    let svc = svc.inner().clone();
    settings::remove_favorite(&svc, &id)?;
    svc.reload().await;
    Ok(())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    al.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_config_dir(app: AppHandle, svc: Svcs<'_>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(svc.config_dir().to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}
