// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `aw-manual-daemon` – the desktop host process: tray, popover, hotkeys,
//! notifications and the IPC server used by the CLI.
//!
//! Options:
//! - `--background`: start without showing the popover (used by autostart and the CLI)
//! - `--headless`: run without any UI (IPC, tracking and writing only)

mod app;
mod commands;
mod hotkeys;
mod icons;
mod mouse;
mod notifications;
mod popover;
mod settings;
mod tray;
mod watcher;

use std::sync::Arc;
use std::time::Duration;

use aw_manual_core::config::{
    load_config_file, ConfigLoad, CONFIG_FILE, DEFAULT_CONFIG_YAML, DEFAULT_FAVORITES_DIR,
};
use aw_manual_core::ipc;
use aw_manual_core::paths;
use aw_manual_core::service::{Headless, Service};

pub struct Options {
    pub background: bool,
    pub first_run: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("aw-manual-daemon {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "aw-manual-daemon {}\n\nUsage: aw-manual-daemon [--background] [--headless]\n\n  \
             --background  start without showing the popover\n  \
             --headless    run without tray, popover and hotkeys (IPC only)",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    let headless = args.iter().any(|a| a == "--headless");
    let background = args.iter().any(|a| a == "--background");

    let config_dir = paths::config_dir();
    let first_run = prepare_config_dir(&config_dir);
    let _log_guard = init_logging();

    let endpoint = match ipc::endpoint() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("cannot determine the IPC endpoint: {e}");
            std::process::exit(1);
        }
    };

    // Single instance: hand over to the running daemon.
    if ipc::call(
        &endpoint,
        "ping",
        serde_json::Value::Null,
        Duration::from_secs(2),
    )
    .is_ok()
    {
        tracing::info!("aw-manual is already running");
        if !headless && !background {
            let _ = ipc::call(
                &endpoint,
                "show_popover",
                serde_json::Value::Null,
                Duration::from_secs(5),
            );
        }
        return;
    }

    if headless {
        run_headless(endpoint);
    } else {
        app::run(
            endpoint,
            Options {
                background,
                first_run,
            },
        );
    }
}

/// Creates the configuration directory with a commented default
/// `config.yaml` on first start. Returns true if it was created.
fn prepare_config_dir(dir: &std::path::Path) -> bool {
    let file = dir.join(CONFIG_FILE);
    if file.exists() {
        return false;
    }
    std::fs::create_dir_all(dir.join(DEFAULT_FAVORITES_DIR)).is_ok()
        && std::fs::write(&file, DEFAULT_CONFIG_YAML).is_ok()
}

fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::prelude::*;
    let level = match load_config_file(&paths::config_dir().join(CONFIG_FILE)) {
        ConfigLoad::Ok(c) | ConfigLoad::Missing(c) => c.log.level,
        _ => "info".into(),
    };
    let filter =
        tracing_subscriber::EnvFilter::try_from_env("AW_MANUAL_LOG").unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(format!(
                "warn,aw_manual_core={level},aw_manual_daemon={level}"
            ))
        });
    let stderr = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    let log_dir = paths::log_dir();
    let (file_layer, guard) = match std::fs::create_dir_all(&log_dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("aw-manual")
                .filename_suffix("log")
                .max_log_files(7)
                .build(&log_dir)
                .ok();
            match appender {
                Some(a) => {
                    let (nb, guard) = tracing_appender::non_blocking(a);
                    (
                        Some(
                            tracing_subscriber::fmt::layer()
                                .with_ansi(false)
                                .with_writer(nb),
                        ),
                        Some(guard),
                    )
                }
                None => (None, None),
            }
        }
        Err(_) => (None, None),
    };
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stderr)
        .with(file_layer)
        .try_init();
    guard
}

fn run_headless(endpoint: ipc::Endpoint) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let svc = match Service::start(
            paths::config_dir(),
            &paths::database_path(),
            Headless {
                shutdown: shutdown.clone(),
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("cannot start: {e:#}");
                std::process::exit(1);
            }
        };
        let listener = match ipc::bind(&endpoint) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("cannot listen on {endpoint}: {e}");
                std::process::exit(1);
            }
        };
        tracing::info!("aw-manual daemon (headless) listening on {endpoint}");
        watcher::spawn(svc.clone());
        tokio::spawn(ipc::serve(listener, svc.clone()));
        tokio::select! {
            _ = shutdown.notified() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    });
}
