//! Tauri application setup and wiring.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use aw_manual_core::engine::ErrorCode;
use aw_manual_core::gap::{self, Gap, GapActivity, GapReason};
use aw_manual_core::ipc::{self, Endpoint, IpcError, NavCommand};
use aw_manual_core::model::Source;
use aw_manual_core::paths;
use aw_manual_core::reminders::{self, ReminderState};
use aw_manual_core::service::{Service, Ui};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::{commands, hotkeys, lock, notifications, popover, tray, watcher, Options};

pub type Svc = Service<TauriUi>;

/// The UI side of the service, backed by Tauri windows.
pub struct TauriUi {
    app: AppHandle,
}

fn unavailable(e: impl std::fmt::Display) -> IpcError {
    IpcError::new(ErrorCode::Internal, e.to_string())
}

impl Ui for TauriUi {
    fn popover(&self, source: Source) -> Result<Value, IpcError> {
        let action = popover::open_or_confirm(&self.app, source).map_err(unavailable)?;
        Ok(json!({ "action": action }))
    }

    fn show_popover(&self) -> Result<Value, IpcError> {
        popover::show(&self.app, "launch").map_err(unavailable)?;
        Ok(json!({ "action": "opened" }))
    }

    fn nav(&self, command: NavCommand) -> Result<Value, IpcError> {
        let handled = popover::nav(&self.app, command).map_err(unavailable)?;
        Ok(json!({ "handled": handled }))
    }

    fn settings(&self) -> Result<Value, IpcError> {
        open_settings(&self.app).map_err(unavailable)?;
        Ok(json!({}))
    }

    fn quit(&self) {
        self.app.exit(0);
    }
}

pub fn open_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    let w = tauri::WebviewWindowBuilder::new(
        app,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("aw-manual Settings")
    .inner_size(860.0, 640.0)
    .min_inner_size(640.0, 480.0)
    .visible(true)
    .build()?;
    w.set_focus()?;
    Ok(())
}

/// Errors from registering hotkeys, shown in the settings window.
#[derive(Default)]
pub struct HotkeyErrors(pub Mutex<Vec<String>>);

pub fn run(endpoint: Endpoint, options: Options) {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(HotkeyErrors::default())
        .manage(hotkeys::MouseActions::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::start_favorite,
            commands::stop,
            commands::resume,
            commands::last_opened,
            commands::set_note,
            commands::log_entry,
            commands::preview_range,
            commands::hide_popover,
            commands::open_settings,
            commands::get_settings,
            commands::save_settings,
            commands::add_favorite,
            commands::remove_favorite,
            commands::set_autostart,
            commands::open_config_dir,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            // Service::start spawns Tokio tasks: enter Tauri's runtime.
            let rt = tauri::async_runtime::handle();
            let _guard = rt.inner().enter();
            let svc = Service::start(
                paths::config_dir(),
                &paths::database_path(),
                TauriUi {
                    app: handle.clone(),
                },
            )
            .map_err(|e| -> Box<dyn std::error::Error> {
                format!("cannot start aw-manual: {e:#}").into()
            })?;
            app.manage(svc.clone());

            let listener = match ipc::bind(&endpoint) {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!(
                        "cannot listen on {endpoint}: {e} (is another instance running?)"
                    );
                    std::process::exit(1);
                }
            };

            popover::create(&handle)?;
            tray::create(&handle, &svc)?;
            hotkeys::apply(&handle, &svc.config());
            watcher::spawn(svc.clone());
            spawn_change_listener(handle.clone(), svc.clone());
            spawn_tray_ticker(handle.clone(), svc.clone());
            spawn_reminders(handle.clone(), svc.clone());

            // Serve IPC only once the windows exist.
            tokio::spawn(ipc::serve(listener, svc.clone()));
            tracing::info!("aw-manual daemon listening on {endpoint}");

            spawn_lock_watcher(handle.clone(), svc.clone());

            // After a (re-)start of aw-manual or the computer: ask what
            // happened while it was stopped and what to continue with.
            let restored = svc.lock_engine().take_restored();
            let previous_alive = svc.take_previous_alive();
            let mut asked = false;
            match (svc.config().notifications.after_restart, options.first_run) {
                (None, _) => tracing::info!("restart prompt disabled (notifications.after_restart)"),
                (Some(_), true) => {}
                (Some(min), false) => {
                    let now = chrono::Utc::now();
                    match gap::restart_gap(previous_alive, restored.as_ref(), now, min) {
                        None => tracing::info!(
                            "restart prompt: not asking (aw-manual was stopped since {}, minimum {}, nothing was running)",
                            previous_alive.map_or("unknown".into(), |t| t.to_rfc3339()),
                            aw_manual_core::duration::format_duration(min)
                        ),
                        Some(g) => {
                            tracing::info!(
                                "restart prompt: aw-manual was stopped from {} to {} ({} min), asking",
                                g.start.to_rfc3339(),
                                g.end.to_rfc3339(),
                                g.minutes
                            );
                            match popover::ask_gap(&handle, g) {
                                Ok(()) => asked = true,
                                Err(e) => {
                                    tracing::warn!("cannot open the popover: {e}");
                                    if let Some(stopped) = &restored {
                                        notifications::restored(&handle, stopped);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if options.first_run {
                let _ = open_settings(&handle);
            } else if !options.background && !asked {
                let _ = popover::show(&handle, "launch");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the aw-manual application");

    app.run(|_app, event| {
        // Keep running in the tray when all windows are closed.
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}

/// Reacts to state, favorites and configuration changes.
fn spawn_change_listener(app: AppHandle, svc: Arc<Svc>) {
    let mut rx = svc.subscribe();
    tokio::spawn(async move {
        let mut last_config = svc.config();
        loop {
            match rx.recv().await {
                Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
            // Coalesce bursts of changes.
            tokio::time::sleep(Duration::from_millis(30)).await;
            while rx.try_recv().is_ok() {}
            let config = svc.config();
            if config.hotkeys != last_config.hotkeys {
                hotkeys::apply(&app, &config);
            }
            last_config = config;
            tray::update(&app, &svc);
            let _ = app.emit("state-changed", ());
        }
    });
}

/// Updates the text next to the tray icon (elapsed time).
fn spawn_tray_ticker(app: AppHandle, svc: Arc<Svc>) {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_secs(1));
        loop {
            iv.tick().await;
            tray::update_text(&app, &svc);
        }
    });
}

fn spawn_reminders(app: AppHandle, svc: Arc<Svc>) {
    tokio::spawn(async move {
        let mut state = ReminderState::default();
        let mut iv = tokio::time::interval(Duration::from_secs(20));
        loop {
            iv.tick().await;
            let cfg = svc.config().notifications;
            let writer = svc.writer().status();
            let notices = {
                let engine = svc.lock_engine();
                reminders::check(
                    &cfg,
                    engine.running(),
                    engine.idle_since(),
                    &writer,
                    &chrono::Local::now(),
                    &mut state,
                )
            };
            for n in notices {
                if matches!(n, reminders::Notice::LongRunning { .. }) {
                    let _ = svc.lock_engine().mark_long_running_notified();
                }
                notifications::notice(&app, &n);
            }
        }
    });
}

/// After the screen was unlocked: ask what the user did while it was locked.
fn spawn_lock_watcher(app: AppHandle, svc: Arc<Svc>) {
    tokio::spawn(async move {
        let mut probe = lock::Probe::new().await;
        let readings = probe.read().await;
        tracing::info!(
            "screen lock detection: {}",
            readings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
        let mut locked_since: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut iv = tokio::time::interval(Duration::from_secs(2));
        loop {
            iv.tick().await;
            let readings = probe.read().await;
            let Some(locked) = lock::locked(&readings) else {
                continue;
            };
            match (locked, locked_since) {
                (true, None) => {
                    let by: Vec<_> = readings
                        .iter()
                        .filter(|r| r.value == Ok(true))
                        .map(|r| r.source.as_str())
                        .collect();
                    tracing::info!("screen locked (detected by: {})", by.join(", "));
                    locked_since = Some(chrono::Utc::now());
                }
                (false, Some(since)) => {
                    locked_since = None;
                    let now = chrono::Utc::now();
                    let secs = (now - since).num_seconds();
                    let Some(min) = svc.config().notifications.after_unlock else {
                        tracing::info!("screen unlocked after {secs}s; prompt disabled (notifications.after_unlock)");
                        continue;
                    };
                    if (now - since).to_std().unwrap_or_default() < min {
                        tracing::info!(
                            "screen unlocked after {secs}s; not asking (minimum {})",
                            aw_manual_core::duration::format_duration(min)
                        );
                        continue;
                    }
                    let last = svc
                        .lock_engine()
                        .running()
                        .map(|r| GapActivity::new(&r.activity, r.note.as_deref()));
                    let gap = Gap::new(GapReason::Unlock, since, now, last);
                    if gap.range.is_none() {
                        tracing::info!(
                            "screen unlocked after {secs}s; not asking (shorter than a minute)"
                        );
                        continue;
                    }
                    tracing::info!("screen unlocked after {secs}s; asking what happened");
                    if let Err(e) = popover::ask_gap(&app, gap) {
                        tracing::warn!("cannot open the popover: {e}");
                    }
                }
                _ => {}
            }
        }
    });
}
