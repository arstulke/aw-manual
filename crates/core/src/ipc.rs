//! IPC between the daemon and its clients (CLI, OpenAction plugin).
//!
//! Transport: a Unix domain socket (Linux, macOS) or a named pipe (Windows).
//! Protocol: newline-delimited JSON.
//!
//! ```text
//! → {"v":1,"id":1,"method":"start","params":{"target":{"favorite":"acme/dev"},"source":"cli"}}
//! ← {"id":1,"ok":true,"result":{...}}
//! ← {"id":1,"ok":false,"error":{"code":"not_found","message":"unknown favorite \"x\""}}
//! ```
//!
//! The `watch` method keeps the connection open and pushes
//! `{"event":"status","data":{...}}` lines after every state change.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::{ErrorCode, Target};
use crate::model::Source;

/// Protocol version. Bumped on incompatible changes.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
}

impl IpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        IpcError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<crate::engine::EngineError> for IpcError {
    fn from(e: crate::engine::EngineError) -> Self {
        IpcError {
            code: e.code,
            message: e.message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Push {
    pub event: String,
    pub data: Value,
}

// --- Parameters of the individual methods -----------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartParams {
    pub target: Target,
    #[serde(default)]
    pub note: Option<String>,
    pub source: Source,
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceParams {
    pub source: Source,
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteParams {
    pub note: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogParams {
    pub target: Target,
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub note: Option<String>,
    pub source: Source,
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NavCommand {
    Next,
    Prev,
    Confirm,
    Back,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavParams {
    pub command: NavCommand,
}

// --- Socket location ----------------------------------------------------------

/// Where the daemon listens. `AW_MANUAL_SOCKET` overrides the default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Unix domain socket path.
    Path(PathBuf),
    /// Windows named pipe name (without `\\.\pipe\`).
    Pipe(String),
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Path(p) => write!(f, "{}", p.display()),
            Endpoint::Pipe(n) => write!(f, r"\\.\pipe\{n}"),
        }
    }
}

pub fn endpoint() -> std::io::Result<Endpoint> {
    if let Ok(v) = std::env::var("AW_MANUAL_SOCKET") {
        if !v.is_empty() {
            return Ok(if cfg!(windows) {
                Endpoint::Pipe(v)
            } else {
                Endpoint::Path(PathBuf::from(v))
            });
        }
    }
    default_endpoint()
}

#[cfg(windows)]
fn default_endpoint() -> std::io::Result<Endpoint> {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let user: String = user
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Ok(Endpoint::Pipe(format!("aw-manual-{user}")))
}

#[cfg(unix)]
fn default_endpoint() -> std::io::Result<Endpoint> {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let dir = match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d).join("aw-manual"),
        None => std::env::temp_dir().join(format!("aw-manual-{uid}")),
    };
    Ok(Endpoint::Path(dir.join("daemon.sock")))
}

/// Creates the socket directory with owner-only permissions.
#[cfg(unix)]
pub fn prepare_socket_dir(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let meta = std::fs::metadata(dir)?;
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    if meta.uid() != uid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("socket directory {} belongs to another user", dir.display()),
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn to_name(ep: &Endpoint) -> std::io::Result<interprocess::local_socket::Name<'static>> {
    use interprocess::local_socket::prelude::*;
    match ep {
        Endpoint::Path(p) => p
            .clone()
            .to_fs_name::<interprocess::local_socket::GenericFilePath>(),
        Endpoint::Pipe(n) => n
            .clone()
            .to_ns_name::<interprocess::local_socket::GenericNamespaced>(),
    }
}

// --- Client --------------------------------------------------------------------

#[derive(Debug)]
pub enum ClientError {
    /// The daemon is not running (or not reachable).
    NotRunning(String),
    /// The daemon answered with an error.
    Remote(IpcError),
    /// Communication failed or timed out.
    Io(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NotRunning(e) => write!(f, "the aw-manual daemon is not running ({e})"),
            ClientError::Remote(e) => f.write_str(&e.message),
            ClientError::Io(e) => write!(f, "communication with the daemon failed: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// A synchronous connection to the daemon.
pub struct Client {
    reader: BufReader<interprocess::local_socket::Stream>,
    next_id: u64,
}

impl Client {
    pub fn connect(ep: &Endpoint) -> Result<Client, ClientError> {
        use interprocess::local_socket::prelude::*;
        let name = to_name(ep).map_err(|e| ClientError::NotRunning(e.to_string()))?;
        let stream = interprocess::local_socket::Stream::connect(name)
            .map_err(|e| ClientError::NotRunning(e.to_string()))?;
        Ok(Client {
            reader: BufReader::new(stream),
            next_id: 1,
        })
    }

    fn send(&mut self, method: &str, params: Value) -> Result<u64, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = Request {
            v: PROTOCOL_VERSION,
            id,
            method: method.to_string(),
            params,
        };
        let mut line = serde_json::to_string(&req).map_err(|e| ClientError::Io(e.to_string()))?;
        line.push('\n');
        self.reader
            .get_mut()
            .write_all(line.as_bytes())
            .map_err(|e| ClientError::Io(e.to_string()))?;
        self.reader
            .get_mut()
            .flush()
            .map_err(|e| ClientError::Io(e.to_string()))?;
        Ok(id)
    }

    fn read_line(&mut self) -> Result<String, ClientError> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(|e| ClientError::Io(e.to_string()))?;
        if n == 0 {
            return Err(ClientError::Io("the daemon closed the connection".into()));
        }
        Ok(line)
    }

    /// Sends a request and waits for its response.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.send(method, params)?;
        loop {
            let line = self.read_line()?;
            let resp: Response = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(_) => continue, // a push message; ignore
            };
            if resp.id != id {
                continue;
            }
            return if resp.ok {
                Ok(resp.result.unwrap_or(Value::Null))
            } else {
                Err(ClientError::Remote(resp.error.unwrap_or(IpcError::new(
                    ErrorCode::Internal,
                    "unknown error",
                ))))
            };
        }
    }

    /// Subscribes to status pushes; calls `f` for the current status and each change.
    /// Returns when `f` returns false or the connection ends.
    pub fn watch(&mut self, mut f: impl FnMut(Value) -> bool) -> Result<(), ClientError> {
        let first = self.call("watch", Value::Null)?;
        if !f(first) {
            return Ok(());
        }
        loop {
            let line = self.read_line()?;
            if let Ok(p) = serde_json::from_str::<Push>(&line) {
                if p.event == "status" && !f(p.data) {
                    return Ok(());
                }
            }
        }
    }
}

/// Connects and performs a single call, with a timeout.
pub fn call(
    ep: &Endpoint,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, ClientError> {
    let ep = ep.clone();
    let method = method.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = Client::connect(&ep).and_then(|mut c| c.call(&method, params));
        let _ = tx.send(r);
    });
    rx.recv_timeout(timeout)
        .unwrap_or_else(|_| Err(ClientError::Io("the daemon did not respond in time".into())))
}

// --- Server ----------------------------------------------------------------------

/// Handles IPC requests in the daemon.
pub trait Handler: Send + Sync + 'static {
    fn handle(
        &self,
        method: String,
        params: Value,
    ) -> impl std::future::Future<Output = Result<Value, IpcError>> + Send;
    /// Current status for `watch`.
    fn status(&self) -> impl std::future::Future<Output = Value> + Send;
    /// Notifies about state changes for `watch`.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<()>;
}

/// Binds the endpoint. Fails with `AddrInUse` if another daemon is listening.
pub fn bind(ep: &Endpoint) -> std::io::Result<interprocess::local_socket::tokio::Listener> {
    use interprocess::local_socket::ListenerOptions;
    #[cfg(unix)]
    if let Endpoint::Path(p) = ep {
        prepare_socket_dir(p)?;
        if p.exists() {
            // A leftover socket file from a crashed daemon? Only remove it if nobody answers.
            use interprocess::local_socket::prelude::*;
            match interprocess::local_socket::Stream::connect(to_name(ep)?) {
                Ok(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        "another daemon is running",
                    ))
                }
                Err(_) => std::fs::remove_file(p)?,
            }
        }
    }
    let listener = ListenerOptions::new().name(to_name(ep)?).create_tokio()?;
    #[cfg(unix)]
    if let Endpoint::Path(p) = ep {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(listener)
}

/// Accepts connections forever.
pub async fn serve<H: Handler>(
    listener: interprocess::local_socket::tokio::Listener,
    handler: std::sync::Arc<H>,
) {
    use interprocess::local_socket::tokio::prelude::*;
    loop {
        match listener.accept().await {
            Ok(conn) => {
                let h = handler.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(conn, h).await {
                        tracing::debug!("IPC connection ended: {e}");
                    }
                });
            }
            Err(e) => {
                tracing::warn!("IPC accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn write_json<W: tokio::io::AsyncWrite + Unpin, T: Serialize>(
    w: &mut W,
    v: &T,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut line = serde_json::to_string(v).map_err(std::io::Error::other)?;
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    w.flush().await
}

async fn handle_conn<H: Handler>(
    conn: interprocess::local_socket::tokio::Stream,
    handler: std::sync::Arc<H>,
) -> std::io::Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let (recv, mut send) = tokio::io::split(conn);
    let mut lines = BufReader::new(recv).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response {
                    id: 0,
                    ok: false,
                    result: None,
                    error: Some(IpcError::new(
                        ErrorCode::BadRequest,
                        format!("malformed request: {e}"),
                    )),
                };
                write_json(&mut send, &resp).await?;
                continue;
            }
        };
        if req.v != PROTOCOL_VERSION {
            let resp = Response {
                id: req.id,
                ok: false,
                result: None,
                error: Some(IpcError::new(
                    ErrorCode::BadRequest,
                    format!(
                        "unsupported protocol version {} (daemon speaks {PROTOCOL_VERSION})",
                        req.v
                    ),
                )),
            };
            write_json(&mut send, &resp).await?;
            continue;
        }
        if req.method == "watch" {
            let mut rx = handler.subscribe();
            let resp = Response {
                id: req.id,
                ok: true,
                result: Some(handler.status().await),
                error: None,
            };
            write_json(&mut send, &resp).await?;
            loop {
                tokio::select! {
                    r = rx.recv() => match r {
                        Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let push = Push { event: "status".into(), data: handler.status().await };
                            write_json(&mut send, &push).await?;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                    },
                    l = lines.next_line() => match l? {
                        None => return Ok(()),
                        Some(_) => {} // ignore further input while watching
                    },
                }
            }
        }
        let resp = match handler.handle(req.method, req.params).await {
            Ok(v) => Response {
                id: req.id,
                ok: true,
                result: Some(v),
                error: None,
            },
            Err(e) => Response {
                id: req.id,
                ok: false,
                result: None,
                error: Some(e),
            },
        };
        write_json(&mut send, &resp).await?;
    }
    Ok(())
}

/// Parses method parameters.
pub fn params<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, IpcError> {
    serde_json::from_value(v)
        .map_err(|e| IpcError::new(ErrorCode::BadRequest, format!("invalid parameters: {e}")))
}
