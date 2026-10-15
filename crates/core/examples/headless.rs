//! Minimal headless daemon (service + IPC, no UI), for development and tests.

use std::sync::Arc;

use aw_manual_core::service::{Headless, Service};
use aw_manual_core::{ipc, paths};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let svc = Service::start(
        paths::config_dir(),
        &paths::database_path(),
        Headless {
            shutdown: shutdown.clone(),
        },
    )?;
    let ep = ipc::endpoint()?;
    let listener = ipc::bind(&ep)?;
    eprintln!("listening on {ep}");
    tokio::spawn(ipc::serve(listener, svc));
    shutdown.notified().await;
    Ok(())
}
