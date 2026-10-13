//! Applies external changes of `config.yaml` and the favorites folders live.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aw_manual_core::favorites::is_favorites_file;
use aw_manual_core::service::{Service, Ui};
use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecursiveMode, Watcher};

/// Quiet period after the last event before reloading.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Starts watching the configuration directory and all favorites folders.
/// Must be called inside a Tokio runtime.
pub fn spawn<U: Ui>(svc: Arc<Service<U>>) {
    let rt = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name("aw-manual-watcher".into())
        .spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                // Opening and reading files (e.g. by our own reload) is no change;
                // forwarding it would make every reload trigger the next one.
                if let Ok(e) = &res {
                    if e.kind.is_access()
                        && e.kind != EventKind::Access(AccessKind::Close(AccessMode::Write))
                    {
                        return;
                    }
                }
                let _ = tx.send(res);
            });
            let mut watcher = match watcher {
                Ok(w) => w,
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
                        let _ = watcher.unwatch(d);
                    }
                    watched.clear();
                    for d in wanted {
                        match watcher.watch(&d, RecursiveMode::NonRecursive) {
                            Ok(()) => watched.push(d),
                            Err(e) => tracing::warn!("cannot watch {}: {e}", d.display()),
                        }
                    }
                }
                let first = match rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(r) => r,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                };
                // Debounce: collect events until none arrives for DEBOUNCE.
                let mut relevant = false;
                let mut next = Some(first);
                while let Some(r) = next {
                    match r {
                        Ok(e) => relevant |= e.paths.iter().any(|p| is_relevant(p)),
                        Err(e) => tracing::warn!("file watcher error: {e}"),
                    }
                    next = rx.recv_timeout(DEBOUNCE).ok();
                }
                if relevant {
                    tracing::debug!("configuration changed on disk, reloading");
                    rt.block_on(svc.reload());
                }
            }
        })
        .expect("watcher thread can be spawned");
}

fn is_relevant(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    is_favorites_file(name)
        || name == aw_manual_core::config::CONFIG_FILE
        || path.is_dir()
        || !path.exists()
}
