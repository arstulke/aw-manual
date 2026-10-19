//! Hardware input devices (see `aw_manual_core::devices`): one reader thread
//! per enabled device, restarted when the configuration changes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use aw_manual_core::config::Config;
use aw_manual_core::devices::{self, DeviceAction, DeviceConfig};
use aw_manual_core::model::Source;
use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::app::Svc;
use crate::popover;

struct Reader {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// The running reader threads.
#[derive(Default)]
pub struct Devices(Mutex<Vec<Reader>>);

/// Starts readers for the enabled devices of the configuration, replacing the
/// running ones.
pub fn apply(app: &AppHandle, cfg: &Config) {
    let state = app.state::<Devices>();
    let mut readers = state.0.lock().unwrap();
    for r in readers.iter() {
        r.stop.store(true, Ordering::Relaxed);
    }
    // The threads notice within 100 ms.
    for r in readers.drain(..) {
        let _ = r.thread.join();
    }
    for device in cfg.devices.iter().filter(|d| d.enabled) {
        let stop = Arc::new(AtomicBool::new(false));
        let (app, device, flag) = (app.clone(), device.clone(), stop.clone());
        let name = format!("device-{}", device.driver);
        match std::thread::Builder::new().name(name).spawn(move || {
            devices::runner::run(&device, &flag, |input| on_input(&app, &device, input))
        }) {
            Ok(thread) => readers.push(Reader { stop, thread }),
            Err(e) => tracing::warn!("cannot start the device reader: {e}"),
        }
    }
}

fn on_input(app: &AppHandle, device: &DeviceConfig, input: &str) {
    app.state::<Arc<Svc>>()
        .publish_device_input(&device.driver, input);
    let Some(action) = device.action(input).cloned() else {
        return;
    };
    let app = app.clone();
    let driver = device.driver.clone();
    // Leave the reader thread quickly.
    tauri::async_runtime::spawn(async move {
        if let Err(e) = trigger(&app, &action) {
            tracing::info!("{driver} action {action} failed: {e}");
        }
    });
}

fn trigger(app: &AppHandle, action: &DeviceAction) -> Result<(), String> {
    let svc = app.state::<Arc<Svc>>().inner().clone();
    let source = Source::Device;
    match action {
        DeviceAction::Popover => popover::open_or_confirm(app, source)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        DeviceAction::OpenPopover => {
            if popover::is_visible(app) {
                return Ok(());
            }
            popover::show(app, popover::HOVER_TRIGGER).map_err(|e| e.to_string())
        }
        DeviceAction::ClosePopover => popover::close_if_opened_by_hover(app)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        DeviceAction::Stop => svc
            .call("stop", json!({"source": source}))
            .map(|_| ())
            .map_err(|e| e.message),
        DeviceAction::Resume => svc
            .call("resume", json!({"source": source}))
            .map(|_| ())
            .map_err(|e| e.message),
        DeviceAction::Nav(command) => popover::nav(app, *command)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        DeviceAction::Favorite(id) => popover::start_or_ask(app, id, source, "device"),
    }
}
