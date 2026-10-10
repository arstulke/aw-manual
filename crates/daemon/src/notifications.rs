//! Native notifications. On Linux, notifications carry actions (Resume,
//! Stop, …) via the freedesktop notification service; on macOS and Windows
//! they are shown without actions and the tray menu offers the same commands.

use std::sync::Arc;

use aw_manual_core::format::{format_clock, format_elapsed, ElapsedFormat, PATH_SEPARATOR};
use aw_manual_core::model::{Source, Stopped};
use aw_manual_core::reminders::Notice;
use chrono::Local;
use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::app::Svc;
use crate::popover;

#[derive(Clone, Copy)]
enum Action {
    Resume,
    Stop,
    Continue,
    OpenPopover,
}

impl Action {
    fn id(self) -> &'static str {
        match self {
            Action::Resume => "resume",
            Action::Stop => "stop",
            Action::Continue => "continue",
            Action::OpenPopover => "open",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Action::Resume => "Resume",
            Action::Stop => "Stop",
            Action::Continue => "Continue",
            Action::OpenPopover => "Open popover",
        }
    }
}

fn run_action(app: &AppHandle, id: &str) {
    let svc = app.state::<Arc<Svc>>().inner().clone();
    let r = match id {
        "resume" => svc
            .call("resume", json!({"source": Source::Tray}))
            .map(|_| ())
            .map_err(|e| e.message),
        "stop" => svc
            .call("stop", json!({"source": Source::Tray}))
            .map(|_| ())
            .map_err(|e| e.message),
        "open" | "default" => popover::show(app, "notification").map_err(|e| e.to_string()),
        _ => Ok(()),
    };
    if let Err(e) = r {
        tracing::warn!("notification action \"{id}\" failed: {e}");
    }
}

#[cfg(target_os = "linux")]
fn show(app: &AppHandle, title: &str, body: &str, actions: &[Action]) {
    let app = app.clone();
    let (title, body) = (title.to_string(), body.to_string());
    let actions = actions.to_vec();
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.appname("aw-manual")
            .summary(&title)
            .body(&body)
            .icon("appointment-soon");
        for a in &actions {
            n.action(a.id(), a.label());
        }
        match n.show() {
            Ok(handle) => {
                if !actions.is_empty() {
                    handle.wait_for_action(|id| run_action(&app, id));
                }
            }
            Err(e) => tracing::warn!("cannot show a notification: {e}"),
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn show(app: &AppHandle, title: &str, body: &str, _actions: &[Action]) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("cannot show a notification: {e}");
    }
}

/// After a restart: the previously running activity ended with its last heartbeat.
pub fn restored(app: &AppHandle, stopped: &Stopped) {
    let svc = app.state::<Arc<Svc>>();
    let fmt = svc.config().tray.time_format;
    let body = format!(
        "Last activity: {} (ended {})",
        stopped.activity.path.join(PATH_SEPARATOR),
        format_clock(&stopped.ended.with_timezone(&Local), fmt)
    );
    let hint = if cfg!(target_os = "linux") {
        ""
    } else {
        "\nUse “Resume” in the tray menu to continue it."
    };
    show(
        app,
        "aw-manual",
        &format!("{body}{hint}"),
        &[Action::Resume],
    );
}

pub fn notice(app: &AppHandle, n: &Notice) {
    match n {
        Notice::Idle { minutes } => show(
            app,
            "No activity running",
            &format!(
                "Nothing has been tracked for {}.",
                format_elapsed(minutes * 60, ElapsedFormat::HoursMinutes)
            ),
            &[Action::OpenPopover],
        ),
        Notice::LongRunning { path, minutes, .. } => show(
            app,
            "Still working on this?",
            &format!(
                "{} has been running for {}.",
                path.join(PATH_SEPARATOR),
                format_elapsed(minutes * 60, ElapsedFormat::HoursMinutes)
            ),
            &[Action::Continue, Action::Stop, Action::OpenPopover],
        ),
        Notice::Offline { pending } => show(
            app,
            "aw-server unreachable",
            &format!("Entries are buffered locally ({pending} pending) and sent when the server is back."),
            &[],
        ),
    }
}
