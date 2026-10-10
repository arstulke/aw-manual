//! The daemon's core: configuration, favorites, engine and writer, exposed
//! through IPC. The desktop host process adds the UI through [`Ui`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::config::{load_config_file, Config, ConfigLoad, CONFIG_FILE};
use crate::engine::{Change, Engine, EngineSettings, ErrorCode, Target};
use crate::favorites::{load_favorites, FavoritesCache, FavoritesLoad, FileReport};
use crate::ipc::{
    self, IpcError, LogParams, NavCommand, NavParams, NoteParams, SourceParams, StartParams,
};
use crate::model::Source;
use crate::store::Store;
use crate::writer::{ServerTarget, Writer};
use crate::yaml::YamlError;

/// UI actions that only the desktop host can perform.
pub trait Ui: Send + Sync + 'static {
    /// Opens the popover, or confirms (scroll mode) if it is already open.
    fn popover(&self, source: Source) -> Result<Value, IpcError>;
    /// Shows the popover without confirming anything (second instance started).
    fn show_popover(&self) -> Result<Value, IpcError>;
    fn nav(&self, command: NavCommand) -> Result<Value, IpcError>;
    fn settings(&self) -> Result<Value, IpcError>;
    fn quit(&self);
}

/// UI for headless operation (no popover, no settings window).
pub struct Headless {
    pub shutdown: Arc<tokio::sync::Notify>,
}

impl Ui for Headless {
    fn popover(&self, _: Source) -> Result<Value, IpcError> {
        Err(IpcError::new(
            ErrorCode::Unavailable,
            "the daemon runs headless; there is no popover",
        ))
    }
    fn show_popover(&self) -> Result<Value, IpcError> {
        Err(IpcError::new(
            ErrorCode::Unavailable,
            "the daemon runs headless; there is no popover",
        ))
    }
    fn nav(&self, _: NavCommand) -> Result<Value, IpcError> {
        Err(IpcError::new(
            ErrorCode::Unavailable,
            "the daemon runs headless; there is no popover",
        ))
    }
    fn settings(&self) -> Result<Value, IpcError> {
        Err(IpcError::new(
            ErrorCode::Unavailable,
            "the daemon runs headless; there is no settings window",
        ))
    }
    fn quit(&self) {
        self.shutdown.notify_waiters();
    }
}

/// State of `config.yaml`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ConfigState {
    /// The file doesn't exist; defaults apply.
    Missing,
    Ok,
    /// The file is invalid; the last valid configuration stays active.
    Invalid {
        errors: Vec<crate::favorites::FileError>,
    },
}

pub struct Loaded {
    pub config: Config,
    pub config_state: ConfigState,
    pub favorites: FavoritesLoad,
    cache: FavoritesCache,
}

pub struct Service<U: Ui> {
    config_dir: PathBuf,
    hostname: String,
    engine: Mutex<Engine>,
    writer: Writer,
    loaded: RwLock<Loaded>,
    changes: broadcast::Sender<()>,
    ui: U,
}

fn yaml_errors(errs: Vec<YamlError>) -> Vec<crate::favorites::FileError> {
    errs.into_iter().map(Into::into).collect()
}

fn read_config(config_dir: &Path, previous: Option<&Config>) -> (Config, ConfigState) {
    match load_config_file(&config_dir.join(CONFIG_FILE)) {
        ConfigLoad::Missing(c) => (c, ConfigState::Missing),
        ConfigLoad::Ok(c) => (c, ConfigState::Ok),
        ConfigLoad::Invalid(e) => (
            previous.cloned().unwrap_or_default(),
            ConfigState::Invalid {
                errors: yaml_errors(e),
            },
        ),
        ConfigLoad::Unreadable(e) => (
            previous.cloned().unwrap_or_default(),
            ConfigState::Invalid {
                errors: vec![crate::favorites::FileError {
                    line: None,
                    message: e,
                }],
            },
        ),
    }
}

fn engine_settings(cfg: &Config, hostname: &str) -> EngineSettings {
    EngineSettings {
        bucket: cfg.bucket_id(hostname),
        heartbeat_interval: cfg.server.heartbeat_interval,
    }
}

fn server_target(cfg: &Config, hostname: &str) -> ServerTarget {
    ServerTarget {
        url: cfg.server.url.clone(),
        bucket_type: cfg.server.bucket_type.clone(),
        bucket: cfg.bucket_id(hostname),
    }
}

impl<U: Ui> Service<U> {
    /// Loads configuration and state and spawns the background tasks
    /// (writer, heartbeat ticker). Must be called inside a Tokio runtime.
    pub fn start(config_dir: PathBuf, database: &Path, ui: U) -> anyhow::Result<Arc<Self>> {
        let hostname = crate::paths::hostname();
        let (config, config_state) = read_config(&config_dir, None);
        let mut cache = FavoritesCache::default();
        let favorites = load_favorites(&config.favorite_dirs(&config_dir), Some(&mut cache));
        let store = Store::open(database)?;
        let writer = Writer::new(
            store.clone(),
            server_target(&config, &hostname),
            hostname.clone(),
        );
        let mut engine = Engine::new(
            store,
            engine_settings(&config, &hostname),
            Arc::new(favorites.tree.clone()),
            Utc::now(),
        )?;
        let w = writer.clone();
        engine.set_queue_listener(move || w.notify());
        let (changes, _) = broadcast::channel(64);

        let svc = Arc::new(Service {
            config_dir,
            hostname,
            engine: Mutex::new(engine),
            writer: writer.clone(),
            loaded: RwLock::new(Loaded {
                config,
                config_state,
                favorites,
                cache,
            }),
            changes,
            ui,
        });

        tokio::spawn(writer.run());

        // Forward engine and writer changes.
        let mut engine_rx = svc.lock_engine().subscribe();
        let tx = svc.changes.clone();
        tokio::spawn(async move {
            while !matches!(
                engine_rx.recv().await,
                Err(broadcast::error::RecvError::Closed)
            ) {
                let _ = tx.send(());
            }
        });
        let mut writer_rx = svc.writer.subscribe();
        let tx = svc.changes.clone();
        tokio::spawn(async move {
            while writer_rx.changed().await.is_ok() {
                let _ = tx.send(());
            }
        });

        // Heartbeat ticker.
        let weak = Arc::downgrade(&svc);
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_secs(5));
            loop {
                iv.tick().await;
                let Some(svc) = weak.upgrade() else { break };
                let result = svc.lock_engine().tick(Utc::now());
                if let Err(e) = result {
                    tracing::error!("heartbeat failed: {e}");
                }
            }
        });
        Ok(svc)
    }

    pub fn lock_engine(&self) -> std::sync::MutexGuard<'_, Engine> {
        self.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn loaded(&self) -> std::sync::RwLockReadGuard<'_, Loaded> {
        self.loaded.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn config(&self) -> Config {
        self.loaded().config.clone()
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    pub fn writer(&self) -> &Writer {
        &self.writer
    }

    pub fn ui(&self) -> &U {
        &self.ui
    }

    /// Fires after every change of state, favorites, configuration or connection.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changes.subscribe()
    }

    /// Re-reads `config.yaml` and all favorites files. Invalid files keep
    /// their last valid version active.
    pub async fn reload(&self) -> Value {
        let (cfg, target, settings) = {
            let mut l = self.loaded.write().unwrap_or_else(|e| e.into_inner());
            let (config, state) = read_config(&self.config_dir, Some(&l.config));
            let dirs = config.favorite_dirs(&self.config_dir);
            let favorites = load_favorites(&dirs, Some(&mut l.cache));
            l.config = config;
            l.config_state = state;
            l.favorites = favorites;
            (
                l.config.clone(),
                server_target(&l.config, &self.hostname),
                engine_settings(&l.config, &self.hostname),
            )
        };
        {
            let mut e = self.lock_engine();
            let tree = Arc::new(self.loaded().favorites.tree.clone());
            e.set_favorites(tree);
            if let Err(err) = e.apply_settings(settings, Utc::now()) {
                tracing::error!("cannot apply settings: {err}");
            }
        }
        self.writer.set_target(target).await;
        let _ = self.changes.send(());
        tracing::info!("configuration reloaded (server {})", cfg.server.url);
        self.list_json()
    }

    pub fn status_json(&self) -> Value {
        let url = self.loaded().config.server.url.clone();
        let st = self
            .lock_engine()
            .status(&self.writer.status(), &url, Utc::now());
        serde_json::to_value(st).expect("status serializes")
    }

    pub fn list_json(&self) -> Value {
        let l = self.loaded();
        json!({
            "schema_version": crate::engine::STATUS_SCHEMA,
            "favorites": l.favorites.tree.roots,
            "files": l.favorites.files,
            "warnings": l.favorites.warnings,
            "config": l.config_state,
            "config_dir": self.config_dir,
        })
    }

    pub fn favorite_files(&self) -> Vec<FileReport> {
        self.loaded().favorites.files.clone()
    }

    fn result(&self, change: Change) -> Value {
        json!({ "change": change, "status": self.status_json() })
    }

    /// Executes a request synchronously (all methods except `watch`).
    pub fn call(&self, method: &str, params: Value) -> Result<Value, IpcError> {
        let now = Utc::now();
        match method {
            "ping" => Ok(json!({
                "protocol": ipc::PROTOCOL_VERSION,
                "version": env!("CARGO_PKG_VERSION"),
                "pid": std::process::id(),
            })),
            "status" => Ok(self.status_json()),
            "list" => Ok(self.list_json()),
            "start" => {
                let p: StartParams = ipc::params(params)?;
                let c = self.lock_engine().start(
                    &p.target,
                    p.note.as_deref(),
                    p.source,
                    p.device.as_deref(),
                    now,
                )?;
                Ok(self.result(c))
            }
            "toggle" => {
                let p: StartParams = ipc::params(params)?;
                let c = self.lock_engine().toggle(
                    &p.target,
                    p.note.as_deref(),
                    p.source,
                    p.device.as_deref(),
                    now,
                )?;
                Ok(self.result(c))
            }
            "stop" => {
                let _p: SourceParams = ipc::params(params)?;
                let c = self.lock_engine().stop(now)?;
                Ok(self.result(c))
            }
            "resume" => {
                let p: SourceParams = ipc::params(params)?;
                let c = self
                    .lock_engine()
                    .resume(p.source, p.device.as_deref(), now)?;
                Ok(self.result(c))
            }
            "note" => {
                let p: NoteParams = ipc::params(params)?;
                let c = self
                    .lock_engine()
                    .set_note(p.note.as_deref(), p.source, now)?;
                Ok(self.result(c))
            }
            "log" => {
                let p: LogParams = ipc::params(params)?;
                let c = self.lock_engine().log(
                    &p.target,
                    p.start,
                    p.end,
                    p.note.as_deref(),
                    p.source,
                    p.device.as_deref(),
                    now,
                )?;
                Ok(self.result(c))
            }
            "popover" => {
                let p: SourceParams = ipc::params(if params.is_null() {
                    json!({"source": "cli"})
                } else {
                    params
                })?;
                self.ui.popover(p.source)
            }
            "show_popover" => self.ui.show_popover(),
            "nav" => {
                let p: NavParams = ipc::params(params)?;
                self.ui.nav(p.command)
            }
            "settings" => self.ui.settings(),
            "quit" => {
                self.ui.quit();
                Ok(json!({}))
            }
            other => Err(IpcError::new(
                ErrorCode::BadRequest,
                format!("unknown method \"{other}\""),
            )),
        }
    }

    /// Convenience for UI code: resolves a favorite target.
    pub fn favorite_target(id: &str) -> Target {
        Target::Favorite(id.to_string())
    }
}

impl<U: Ui> ipc::Handler for Service<U> {
    async fn handle(&self, method: String, params: Value) -> Result<Value, IpcError> {
        if method == "reload" {
            return Ok(self.reload().await);
        }
        self.call(&method, params)
    }

    async fn status(&self) -> Value {
        self.status_json()
    }

    fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changes.subscribe()
    }
}
