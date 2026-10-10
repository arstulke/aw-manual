//! Applies external changes of `config.yaml` and the favorites folders live.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aw_manual_core::favorites::is_favorites_file;
use aw_manual_core::service::{Service, Ui};
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;

/// Starts watching the configuration directory and all favorites folders.
/// Must be called inside a Tokio runtime.
pub fn spawn<U: Ui>(svc: Arc<Service<U>>) {
    let rt = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name("aw-manual-watcher".into())
        .spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut debouncer = match new_debouncer(Duration::from_millis(250), tx) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("cannot watch the configuration for changes: {e}");
                    return;
                }
            };
            let mut watched: Vec<PathBuf> = Vec::new();
            loop {
                // (Re)compute the watched folders: they may change with the configuration
                // or appear later (e.g. favorites.d created after start).
                let mut wanted = vec![svc.config_dir().to_path_buf()];
                wanted.extend(svc.config().favorite_dirs(svc.config_dir()));
                wanted.retain(|d| d.is_dir());
                wanted.dedup();
                if wanted != watched {
                    for d in &watched {
                        let _ = debouncer.watcher().unwatch(d);
                    }
                    watched.clear();
                    for d in wanted {
                        match debouncer.watcher().watch(&d, RecursiveMode::NonRecursive) {
                            Ok(()) => watched.push(d),
                            Err(e) => tracing::warn!("cannot watch {}: {e}", d.display()),
                        }
                    }
                }
                match rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(Ok(events)) => {
                        let relevant = events.iter().any(|e| {
                            let name = e
                                .path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or_default();
                            is_favorites_file(name)
                                || name == aw_manual_core::config::CONFIG_FILE
                                || e.path.is_dir()
                                || !e.path.exists()
                        });
                        if relevant {
                            tracing::debug!("configuration changed on disk, reloading");
                            rt.block_on(svc.reload());
                        }
                    }
                    Ok(Err(e)) => tracing::warn!("file watcher error: {e}"),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .expect("watcher thread can be spawned");
}
