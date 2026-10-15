//! Applies queued operations to aw-server, in order, with offline buffering.
//!
//! ## Heartbeats and the server-side heartbeat cache
//! Both aw-server implementations merge a heartbeat into the bucket's *last*
//! event and cache that event in memory. The caches are not reliably
//! invalidated when events are inserted or deleted through the REST API
//! (aw-server-python never invalidates it, aw-server-rust only on inserts).
//! The writer therefore follows these rules:
//! - Every run starts with a heartbeat with `pulsetime = 0`, so it can never
//!   be merged into an earlier event (e.g. when the same activity is resumed
//!   shortly after it was stopped).
//! - Later heartbeats of a run use a pulsetime covering the whole run, so the
//!   event keeps growing even across suspend/resume.
//! - A retroactive entry that does not touch the running activity is
//!   inserted normally; the running event stays the last event and the cache
//!   stays valid.
//! - A retroactive entry overlapping the running activity is written as a
//!   heartbeat (its data always differs from the running event because
//!   `entry` differs), followed by a fresh `pulsetime = 0` heartbeat that
//!   restarts the running activity at the end of the entry. Both replace the
//!   server's cached last event, so no later heartbeat can merge into a stale,
//!   deleted or trimmed event.
//!
//! Retroactive entries are applied idempotently: if the operation is
//! interrupted halfway, replaying it doesn't duplicate events.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{watch, Mutex, Notify};

use crate::aw::{to_secs, AwClient, AwError, AwEvent, NewEvent};
use crate::model::EventData;
use crate::store::{Op, Restart, Store};

/// Connection state shown in the tray and in `aw-manual status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct WriterStatus {
    /// `None` until the first contact attempt.
    pub online: Option<bool>,
    pub pending: usize,
    pub last_error: Option<String>,
    /// Operations the server rejected permanently (kept in the local database).
    pub failed: usize,
}

#[derive(Debug, Clone)]
pub struct ServerTarget {
    pub url: String,
    pub bucket_type: String,
    /// Bucket used for periodic connectivity checks (the current bucket).
    pub bucket: String,
}

struct Inner {
    client: AwClient,
    target: ServerTarget,
    ensured: HashSet<String>,
}

#[derive(Clone)]
pub struct Writer {
    store: Store,
    hostname: String,
    inner: Arc<Mutex<Inner>>,
    wake: Arc<Notify>,
    status_tx: Arc<watch::Sender<WriterStatus>>,
}

const TOLERANCE_MS: i64 = 1;
const IDLE_CHECK: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

fn close(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    (a - b).num_milliseconds().abs() <= TOLERANCE_MS
}

impl Writer {
    pub fn new(store: Store, target: ServerTarget, hostname: String) -> Writer {
        let pending = store.pending().unwrap_or(0);
        let failed = store.dead_letter_count().unwrap_or(0);
        let (status_tx, _) = watch::channel(WriterStatus {
            online: None,
            pending,
            last_error: None,
            failed,
        });
        Writer {
            store,
            hostname,
            inner: Arc::new(Mutex::new(Inner {
                client: AwClient::new(&target.url),
                target,
                ensured: HashSet::new(),
            })),
            wake: Arc::new(Notify::new()),
            status_tx: Arc::new(status_tx),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<WriterStatus> {
        self.status_tx.subscribe()
    }

    pub fn status(&self) -> WriterStatus {
        self.status_tx.borrow().clone()
    }

    /// Signals that new operations were queued.
    pub fn notify(&self) {
        self.refresh_pending();
        self.wake.notify_one();
    }

    /// Switches to another server or bucket type (config change).
    pub async fn set_target(&self, target: ServerTarget) {
        let mut inner = self.inner.lock().await;
        if inner.target.url != target.url {
            inner.client = AwClient::new(&target.url);
            self.status_tx.send_modify(|s| s.online = None);
        }
        inner.target = target;
        inner.ensured.clear();
        drop(inner);
        self.wake.notify_one();
    }

    pub async fn server_url(&self) -> String {
        self.inner.lock().await.target.url.clone()
    }

    fn refresh_pending(&self) {
        let pending = self.store.pending().unwrap_or(0);
        let failed = self.store.dead_letter_count().unwrap_or(0);
        self.status_tx.send_if_modified(|s| {
            let changed = s.pending != pending || s.failed != failed;
            s.pending = pending;
            s.failed = failed;
            changed
        });
    }

    fn set_online(&self, online: bool, error: Option<String>) {
        self.status_tx.send_if_modified(|s| {
            let changed = s.online != Some(online) || s.last_error != error;
            s.online = Some(online);
            s.last_error = error;
            changed
        });
    }

    async fn ensure_bucket(&self, bucket: &str) -> Result<(), AwError> {
        let mut inner = self.inner.lock().await;
        if inner.ensured.contains(bucket) {
            return Ok(());
        }
        inner
            .client
            .ensure_bucket(bucket, &inner.target.bucket_type, &self.hostname)
            .await?;
        inner.ensured.insert(bucket.to_string());
        Ok(())
    }

    async fn client(&self) -> AwClient {
        self.inner.lock().await.client.clone()
    }

    /// Applies one operation. Bucket creation is retried once if the bucket vanished.
    pub async fn apply(&self, op: &Op) -> Result<(), AwError> {
        self.ensure_bucket(op.bucket()).await?;
        match self.apply_once(op).await {
            Err(AwError::BucketMissing) => {
                self.inner.lock().await.ensured.remove(op.bucket());
                self.ensure_bucket(op.bucket()).await?;
                match self.apply_once(op).await {
                    Err(AwError::BucketMissing) => Err(AwError::Rejected(
                        404,
                        "bucket or event not found after creating the bucket".into(),
                    )),
                    r => r,
                }
            }
            r => r,
        }
    }

    async fn apply_once(&self, op: &Op) -> Result<(), AwError> {
        let client = self.client().await;
        match op {
            Op::Heartbeat {
                bucket,
                timestamp,
                data,
                duration,
                pulsetime,
            } => {
                let ev = NewEvent {
                    timestamp: *timestamp,
                    duration: *duration,
                    data: data.to_json(),
                };
                client.heartbeat(bucket, &ev, *pulsetime).await
            }
            Op::Retro {
                bucket,
                start,
                end,
                data,
                restart,
            } => apply_retro(&client, bucket, *start, *end, data, restart.as_ref()).await,
        }
    }

    /// Runs the writer loop forever.
    pub async fn run(self) {
        let mut backoff = Duration::from_secs(1);
        let mut last_contact: Option<std::time::Instant> = None;
        loop {
            self.refresh_pending();
            let next = match self.store.peek() {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("cannot read the offline queue: {e:#}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
            match next {
                None => {
                    // Nothing to write: check connectivity (and create the bucket) now and then.
                    let due = last_contact.is_none_or(|t| t.elapsed() >= IDLE_CHECK)
                        || self.status_tx.borrow().online != Some(true);
                    if due {
                        let bucket = self.inner.lock().await.target.bucket.clone();
                        match self.ensure_bucket(&bucket).await {
                            Ok(()) => {
                                // ensure_bucket is cached; verify reachability explicitly.
                                match self.client().await.info().await {
                                    Ok(_) => {
                                        self.set_online(true, None);
                                        backoff = Duration::from_secs(1);
                                    }
                                    Err(e) => self.set_online(false, Some(e.to_string())),
                                }
                            }
                            Err(e) => self.set_online(false, Some(e.to_string())),
                        }
                        last_contact = Some(std::time::Instant::now());
                    }
                    let wait = if self.status_tx.borrow().online == Some(true) {
                        IDLE_CHECK
                    } else {
                        backoff
                    };
                    if self.status_tx.borrow().online != Some(true) {
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                    let _ = tokio::time::timeout(wait, self.wake.notified()).await;
                }
                Some((id, op)) => match self.apply(&op).await {
                    Ok(()) => {
                        if let Err(e) = self.store.remove(id) {
                            tracing::error!(
                                "cannot remove an applied operation from the queue: {e:#}"
                            );
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                        self.set_online(true, None);
                        last_contact = Some(std::time::Instant::now());
                        backoff = Duration::from_secs(1);
                    }
                    Err(e) if e.is_retryable() => {
                        tracing::warn!("write failed, will retry in {}s: {e}", backoff.as_secs());
                        self.set_online(false, Some(e.to_string()));
                        let _ = tokio::time::timeout(backoff, self.wake.notified()).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                    Err(e) => {
                        tracing::error!("aw-server rejected an operation permanently, it is kept in the local database: {e}");
                        if let Err(e2) = self.store.dead_letter(id, &op, &e.to_string()) {
                            tracing::error!("cannot park the rejected operation: {e2:#}");
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                        self.status_tx
                            .send_modify(|s| s.last_error = Some(e.to_string()));
                    }
                },
            }
        }
    }
}

/// Inserts a retroactive entry; overlapping events of the bucket are trimmed or split.
pub async fn apply_retro(
    client: &AwClient,
    bucket: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    data: &EventData,
    restart: Option<&Restart>,
) -> Result<(), AwError> {
    let retro_json = data.to_json();
    let restart_json = restart.map(|r| r.data.to_json());
    let query_start =
        restart.map_or(start, |r| r.run_start.min(start)) - chrono::Duration::seconds(1);
    let query_end = end + chrono::Duration::seconds(1);

    // Servers may clip listed events to the range: fetch each one by ID for its real extent.
    let listed = client
        .events_between(bucket, query_start, query_end)
        .await?;
    let mut events: Vec<AwEvent> = Vec::new();
    for ev in listed {
        if let Some(id) = ev.id {
            if let Some(full) = client.get_event(bucket, id).await? {
                events.push(full);
            }
        }
    }

    let exists = |ts: DateTime<Utc>, e: DateTime<Utc>, d: &serde_json::Value| {
        events
            .iter()
            .any(|x| close(x.timestamp, ts) && close(x.end(), e) && &x.data == d)
    };

    let mut inserts: Vec<NewEvent> = Vec::new();
    let mut deletes: Vec<i64> = Vec::new();
    let mut running_found = false;
    let add_piece = |ts: DateTime<Utc>,
                     e: DateTime<Utc>,
                     d: &serde_json::Value,
                     inserts: &mut Vec<NewEvent>| {
        if e > ts
            && !exists(ts, e, d)
            && !inserts
                .iter()
                .any(|n| close(n.timestamp, ts) && &n.data == d)
        {
            inserts.push(NewEvent {
                timestamp: ts,
                duration: to_secs(e - ts),
                data: d.clone(),
            });
        }
    };

    for ev in &events {
        let (a, b) = (ev.timestamp, ev.end());
        if ev.data == retro_json && close(a, start) && close(b, end) {
            continue; // already written (replayed operation)
        }
        if let (Some(r), Some(rj)) = (restart, &restart_json) {
            if &ev.data == rj && close(a, r.run_start) {
                // The running activity's event: it ends where the retroactive entry starts.
                running_found = true;
                if a < start {
                    if close(b, start) {
                        continue; // already trimmed (replayed operation)
                    }
                    add_piece(a, start, rj, &mut inserts);
                }
                if let Some(id) = ev.id {
                    deletes.push(id);
                }
                continue;
            }
        }
        if !(a < end && b > start) {
            continue;
        }
        if a < start {
            add_piece(a, start, &ev.data, &mut inserts);
        }
        if b > end {
            add_piece(end, b, &ev.data, &mut inserts);
        }
        if let Some(id) = ev.id {
            deletes.push(id);
        }
    }
    if let (Some(r), Some(rj)) = (restart, &restart_json) {
        if !running_found && r.run_start < start {
            add_piece(r.run_start, start, rj, &mut inserts);
        }
    }

    // Insert the remaining pieces before deleting the originals, so an
    // interrupted operation never loses data.
    client.insert(bucket, &inserts).await?;
    for id in deletes {
        client.delete_event(bucket, id).await?;
    }

    let retro_event = NewEvent {
        timestamp: start,
        duration: to_secs(end - start),
        data: retro_json.clone(),
    };
    match restart {
        None => {
            if !exists(start, end, &retro_json) {
                client.insert(bucket, &[retro_event]).await?;
            }
        }
        Some(r) => {
            if !exists(start, end, &retro_json) {
                client.heartbeat(bucket, &retro_event, 0.0).await?;
            }
            let restart_event = NewEvent {
                timestamp: end,
                duration: crate::engine::FIRST_HEARTBEAT_DURATION,
                data: r.data.to_json(),
            };
            client.heartbeat(bucket, &restart_event, 0.0).await?;
        }
    }
    Ok(())
}
