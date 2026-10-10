//! The tracking state machine. At most one activity runs at any time.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::favorites::{FavoritesTree, Kind};
use crate::model::{trunc_ms, Activity, Entry, EventData, Origin, Run, Source, Stopped};
use crate::store::{Op, PersistedState, Restart, Store};
use crate::writer::WriterStatus;

/// Stable error codes (also used for CLI exit codes and IPC errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Unknown favorite ID.
    NotFound,
    /// The ID belongs to a folder, only leaves can be started.
    IsFolder,
    /// Invalid input (time range, ad-hoc definition, …).
    Invalid,
    /// The command needs a running activity.
    NotRunning,
    /// There is no stopped activity to resume.
    NothingToResume,
    /// Internal failure (e.g. local database).
    Internal,
    /// Unknown IPC method or malformed request.
    BadRequest,
    /// The command is not available (e.g. no popover in headless mode).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineError {
    pub code: ErrorCode,
    pub message: String,
}

impl EngineError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        EngineError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineError {}

impl From<anyhow::Error> for EngineError {
    fn from(e: anyhow::Error) -> Self {
        EngineError::new(ErrorCode::Internal, format!("{e:#}"))
    }
}

/// What to start: a favorite or an ad-hoc definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Favorite(String),
    Adhoc(AdhocActivity),
}

/// A freely defined activity (CLI `--adhoc`, OpenAction Custom Activity).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdhocActivity {
    pub id: String,
    pub title: String,
    /// Folder titles above the activity; the title is appended.
    #[serde(default)]
    pub path: Vec<String>,
    #[serde(default = "default_kind")]
    pub kind: Kind,
    /// `adhoc` (default) or `deck`.
    #[serde(default = "default_origin")]
    pub origin: Origin,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

fn default_kind() -> Kind {
    Kind::Activity
}

fn default_origin() -> Origin {
    Origin::Adhoc
}

/// The running activity as reported to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunningInfo {
    pub id: String,
    pub title: String,
    pub path: Vec<String>,
    pub kind: Kind,
    pub origin: Origin,
    pub note: Option<String>,
    pub source: Source,
    pub start: DateTime<Utc>,
    pub elapsed: i64,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoppedInfo {
    pub id: String,
    pub title: String,
    pub path: Vec<String>,
    pub kind: Kind,
    pub origin: Origin,
    pub note: Option<String>,
    pub ended: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub url: String,
    pub bucket: String,
    pub online: Option<bool>,
    pub pending: usize,
    pub failed: usize,
    pub last_error: Option<String>,
}

/// JSON schema version of [`Status`] and other IPC/CLI JSON output.
pub const STATUS_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub schema_version: u32,
    pub running: Option<RunningInfo>,
    pub last_stopped: Option<StoppedInfo>,
    pub server: ServerInfo,
    pub now: DateTime<Utc>,
}

/// Result of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Started,
    Switched,
    Stopped,
    NoteChanged,
    Logged,
    Unchanged,
}

/// Settings the engine needs from the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineSettings {
    pub bucket: String,
    pub heartbeat_interval: Duration,
}

pub struct Engine {
    store: Store,
    state: PersistedState,
    settings: EngineSettings,
    favorites: Arc<FavoritesTree>,
    changes: broadcast::Sender<()>,
    /// The activity that was running when the daemon stopped unexpectedly.
    restored: Option<Stopped>,
    /// Since when no activity is running (for the idle reminder).
    idle_since: DateTime<Utc>,
    on_queue: Box<dyn Fn() + Send + Sync>,
}

/// Duration (seconds) of the first heartbeat of a run, see [`Engine::clock`].
pub const FIRST_HEARTBEAT_DURATION: f64 = 0.001;

fn normalize_note(note: Option<&str>) -> Option<String> {
    note.map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

impl Engine {
    /// Creates the engine from the persisted state. A run that was still
    /// active (daemon crash, reboot) is not resumed: it ends with its last
    /// heartbeat and becomes the "last stopped" activity.
    pub fn new(
        store: Store,
        settings: EngineSettings,
        favorites: Arc<FavoritesTree>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Engine> {
        let mut state = store.load_state()?;
        let mut restored = None;
        let mut idle_since = now;
        if let Some(run) = state.running.take() {
            let stopped = Stopped {
                activity: run.activity,
                note: run.note,
                ended: run.last_heartbeat,
            };
            idle_since = stopped.ended.min(now);
            restored = Some(stopped.clone());
            state.last_stopped = Some(stopped);
            store.save_state(&state)?;
        }
        let (changes, _) = broadcast::channel(64);
        Ok(Engine {
            store,
            state,
            settings,
            favorites,
            changes,
            restored,
            idle_since,
            on_queue: Box::new(|| {}),
        })
    }

    /// Called whenever operations were queued (wakes the writer).
    pub fn set_queue_listener(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.on_queue = Box::new(f);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.changes.subscribe()
    }

    /// The activity that was running before an unexpected restart (taken once).
    pub fn take_restored(&mut self) -> Option<Stopped> {
        self.restored.take()
    }

    pub fn settings(&self) -> &EngineSettings {
        &self.settings
    }

    /// Applies new settings. If the bucket changes while an activity is
    /// running, the activity ends in the old bucket and continues in the new one.
    pub fn apply_settings(
        &mut self,
        settings: EngineSettings,
        now: DateTime<Utc>,
    ) -> Result<(), EngineError> {
        let bucket_changed = settings.bucket != self.settings.bucket;
        match self.state.running.clone() {
            Some(run) if bucket_changed => {
                let now = self.clock(now);
                let mut ops = vec![self.heartbeat_op(&run, now, false)];
                self.settings = settings;
                let run = Run {
                    start: now,
                    last_heartbeat: now,
                    ..run
                };
                ops.push(self.heartbeat_op(&run, now, true));
                self.state.running = Some(run);
                self.commit(ops)
            }
            _ => {
                self.settings = settings;
                Ok(())
            }
        }
    }

    pub fn favorites(&self) -> &Arc<FavoritesTree> {
        &self.favorites
    }

    pub fn set_favorites(&mut self, favorites: Arc<FavoritesTree>) {
        self.favorites = favorites;
        let _ = self.changes.send(());
    }

    pub fn running(&self) -> Option<&Run> {
        self.state.running.as_ref()
    }

    pub fn last_stopped(&self) -> Option<&Stopped> {
        self.state.last_stopped.as_ref()
    }

    pub fn idle_since(&self) -> DateTime<Utc> {
        self.idle_since
    }

    pub fn resolve(&self, target: &Target) -> Result<Activity, EngineError> {
        match target {
            Target::Favorite(id) => {
                let id = id.trim().trim_matches('/');
                match self.favorites.find(id) {
                    None => Err(EngineError::new(
                        ErrorCode::NotFound,
                        format!("unknown favorite \"{id}\""),
                    )),
                    Some(f) if !f.is_leaf() => Err(EngineError::new(
                        ErrorCode::IsFolder,
                        format!("\"{id}\" is a folder; only leaves can be started"),
                    )),
                    Some(f) => Ok(Activity::from_favorite(f)),
                }
            }
            Target::Adhoc(a) => {
                if a.id.trim().is_empty() {
                    return Err(EngineError::new(
                        ErrorCode::Invalid,
                        "an ad-hoc activity needs an ID",
                    ));
                }
                if a.title.trim().is_empty() {
                    return Err(EngineError::new(
                        ErrorCode::Invalid,
                        "an ad-hoc activity needs a title",
                    ));
                }
                if a.origin == Origin::Config {
                    return Err(EngineError::new(
                        ErrorCode::Invalid,
                        "ad-hoc activities must use origin \"adhoc\" or \"deck\"",
                    ));
                }
                let mut path: Vec<String> = a
                    .path
                    .iter()
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect();
                path.push(a.title.trim().to_string());
                Ok(Activity {
                    id: a.id.trim().to_string(),
                    title: a.title.trim().to_string(),
                    path,
                    kind: a.kind,
                    origin: a.origin,
                    color: a.color.clone(),
                    icon: a.icon.clone(),
                })
            }
        }
    }

    fn pulsetime(&self, run_start: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
        let elapsed = (now - run_start).num_milliseconds().max(0) as f64 / 1000.0;
        elapsed + self.settings.heartbeat_interval.as_secs_f64() * 1.5
    }

    fn heartbeat_op(&self, run: &Run, ts: DateTime<Utc>, first: bool) -> Op {
        Op::Heartbeat {
            bucket: self.settings.bucket.clone(),
            timestamp: ts,
            data: run.event_data(),
            duration: if first { FIRST_HEARTBEAT_DURATION } else { 0.0 },
            pulsetime: if first {
                0.0
            } else {
                self.pulsetime(run.start, ts)
            },
        }
    }

    /// The current time for new operations: truncated to milliseconds and
    /// never before the end of the latest event, so events never overlap
    /// and the running event always ends last.
    fn clock(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        let mut t = trunc_ms(now);
        if let Some(run) = &self.state.running {
            t = t
                .max(run.start + chrono::Duration::milliseconds(1))
                .max(run.last_heartbeat);
        }
        if let Some(s) = &self.state.last_stopped {
            t = t.max(s.ended);
        }
        t
    }

    fn commit(&mut self, ops: Vec<Op>) -> Result<(), EngineError> {
        self.store.commit(&self.state, &ops)?;
        if !ops.is_empty() {
            (self.on_queue)();
        }
        let _ = self.changes.send(());
        Ok(())
    }

    /// Ends the running activity at `now` (final heartbeat).
    fn end_running(
        &mut self,
        now: DateTime<Utc>,
        remember: bool,
        ops: &mut Vec<Op>,
    ) -> Option<Run> {
        let run = self.state.running.take()?;
        ops.push(self.heartbeat_op(&run, now, false));
        if remember {
            self.state.last_stopped = Some(Stopped {
                activity: run.activity.clone(),
                note: run.note.clone(),
                ended: now,
            });
        }
        self.idle_since = now;
        Some(run)
    }

    fn begin(
        &mut self,
        activity: Activity,
        note: Option<String>,
        source: Source,
        device: Option<String>,
        now: DateTime<Utc>,
        ops: &mut Vec<Op>,
    ) {
        let run = Run {
            activity,
            note,
            source,
            device,
            start: now,
            last_heartbeat: now,
            long_running_notified: false,
        };
        ops.push(self.heartbeat_op(&run, now, true));
        self.state.running = Some(run);
    }

    /// Starts an activity. Starting the activity that is already running
    /// changes nothing, unless a different note is given (then the running
    /// activity ends and starts again with the new note).
    pub fn start(
        &mut self,
        target: &Target,
        note: Option<&str>,
        source: Source,
        device: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Change, EngineError> {
        let now = self.clock(now);
        let activity = self.resolve(target)?;
        let note = normalize_note(note);
        if let Some(run) = &self.state.running {
            if run.activity.same_as(&activity) {
                return match &note {
                    None => Ok(Change::Unchanged),
                    Some(n) if run.note.as_ref() == Some(n) => Ok(Change::Unchanged),
                    Some(_) => self.set_note(note.as_deref(), source, now),
                };
            }
        }
        let mut ops = Vec::new();
        let switched = self.end_running(now, true, &mut ops).is_some();
        self.begin(
            activity,
            note,
            source,
            device.map(str::to_string),
            now,
            &mut ops,
        );
        self.commit(ops)?;
        Ok(if switched {
            Change::Switched
        } else {
            Change::Started
        })
    }

    pub fn stop(&mut self, now: DateTime<Utc>) -> Result<Change, EngineError> {
        let now = self.clock(now);
        let mut ops = Vec::new();
        if self.end_running(now, true, &mut ops).is_none() {
            return Err(EngineError::new(
                ErrorCode::NotRunning,
                "no activity is running",
            ));
        }
        self.commit(ops)?;
        Ok(Change::Stopped)
    }

    /// Starts the favorite, or stops it if it is already running.
    pub fn toggle(
        &mut self,
        target: &Target,
        note: Option<&str>,
        source: Source,
        device: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Change, EngineError> {
        let activity = self.resolve(target)?;
        if self
            .state
            .running
            .as_ref()
            .is_some_and(|r| r.activity.same_as(&activity))
        {
            return self.stop(now);
        }
        self.start(target, note, source, device, now)
    }

    /// Resumes the last stopped activity (with its note). If it was a
    /// favorite that still exists, its current title and path are used.
    pub fn resume(
        &mut self,
        source: Source,
        device: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Change, EngineError> {
        let Some(last) = self.state.last_stopped.clone() else {
            return Err(EngineError::new(
                ErrorCode::NothingToResume,
                "there is no stopped activity to resume",
            ));
        };
        let activity = match last.activity.origin {
            Origin::Config => match self.favorites.find(&last.activity.id) {
                Some(f) if f.is_leaf() => Activity::from_favorite(f),
                _ => last.activity.clone(),
            },
            _ => last.activity.clone(),
        };
        if let Some(run) = &self.state.running {
            if run.activity.same_as(&activity) && run.note == last.note {
                return Ok(Change::Unchanged);
            }
        }
        let now = self.clock(now);
        let mut ops = Vec::new();
        let switched = self.end_running(now, true, &mut ops).is_some();
        self.begin(
            activity,
            last.note.clone(),
            source,
            device.map(str::to_string),
            now,
            &mut ops,
        );
        self.commit(ops)?;
        Ok(if switched {
            Change::Switched
        } else {
            Change::Started
        })
    }

    /// Sets a new note: the running activity ends now and starts again with the note.
    pub fn set_note(
        &mut self,
        note: Option<&str>,
        source: Source,
        now: DateTime<Utc>,
    ) -> Result<Change, EngineError> {
        let now = self.clock(now);
        let note = normalize_note(note);
        let Some(run) = self.state.running.clone() else {
            return Err(EngineError::new(
                ErrorCode::NotRunning,
                "no activity is running",
            ));
        };
        if run.note == note {
            return Ok(Change::Unchanged);
        }
        let mut ops = Vec::new();
        self.end_running(now, false, &mut ops);
        self.begin(run.activity, note, source, run.device, now, &mut ops);
        self.commit(ops)?;
        Ok(Change::NoteChanged)
    }

    /// Records a retroactive entry. It takes precedence over overlapping
    /// entries; if it overlaps the running activity, that one restarts at
    /// the end of the entry.
    #[allow(clippy::too_many_arguments)]
    pub fn log(
        &mut self,
        target: &Target,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        note: Option<&str>,
        source: Source,
        device: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Change, EngineError> {
        let activity = self.resolve(target)?;
        let (start, end, now) = (trunc_ms(start), trunc_ms(end), self.clock(now));
        if end <= start {
            return Err(EngineError::new(
                ErrorCode::Invalid,
                "the end of the entry must be after its start",
            ));
        }
        if end > now {
            return Err(EngineError::new(
                ErrorCode::Invalid,
                "a retroactive entry must not end in the future",
            ));
        }
        let note = normalize_note(note);
        let data = EventData::new(&activity, note.as_deref(), source, device, Entry::Retro);
        let mut restart = None;
        if let Some(run) = self.state.running.as_mut() {
            if end > run.start {
                restart = Some(Restart {
                    data: run.event_data(),
                    run_start: run.start,
                });
                run.start = end;
                run.last_heartbeat = end;
            }
        }
        let op = Op::Retro {
            bucket: self.settings.bucket.clone(),
            start,
            end,
            data,
            restart,
        };
        self.commit(vec![op])?;
        Ok(Change::Logged)
    }

    /// Queues a heartbeat for the running activity if one is due.
    pub fn tick(&mut self, now: DateTime<Utc>) -> Result<bool, EngineError> {
        let now = self.clock(now);
        let interval = chrono::Duration::from_std(self.settings.heartbeat_interval)
            .unwrap_or(chrono::Duration::seconds(60));
        let Some(run) = self.state.running.as_ref() else {
            return Ok(false);
        };
        if now - run.last_heartbeat < interval {
            return Ok(false);
        }
        let op = self.heartbeat_op(run, now, false);
        if let Some(run) = self.state.running.as_mut() {
            run.last_heartbeat = now;
        }
        self.store.commit(&self.state, &[op])?;
        (self.on_queue)();
        Ok(true)
    }

    /// Marks the long-running notification as shown for the current run.
    pub fn mark_long_running_notified(&mut self) -> Result<(), EngineError> {
        if let Some(run) = self.state.running.as_mut() {
            run.long_running_notified = true;
            self.store.save_state(&self.state)?;
        }
        Ok(())
    }

    pub fn status(&self, writer: &WriterStatus, server_url: &str, now: DateTime<Utc>) -> Status {
        let running = self.state.running.as_ref().map(|r| RunningInfo {
            id: r.activity.id.clone(),
            title: r.activity.title.clone(),
            path: r.activity.path.clone(),
            kind: r.activity.kind,
            origin: r.activity.origin,
            note: r.note.clone(),
            source: r.source,
            start: r.start,
            elapsed: (now - r.start).num_seconds().max(0),
            color: r.activity.color.clone(),
            icon: r.activity.icon.clone(),
        });
        let last_stopped = self.state.last_stopped.as_ref().map(|s| StoppedInfo {
            id: s.activity.id.clone(),
            title: s.activity.title.clone(),
            path: s.activity.path.clone(),
            kind: s.activity.kind,
            origin: s.activity.origin,
            note: s.note.clone(),
            ended: s.ended,
        });
        Status {
            schema_version: STATUS_SCHEMA,
            running,
            last_stopped,
            server: ServerInfo {
                url: server_url.to_string(),
                bucket: self.settings.bucket.clone(),
                online: writer.online,
                pending: writer.pending,
                failed: writer.failed,
                last_error: writer.last_error.clone(),
            },
            now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::favorites::{merge_parsed, parse_favorites_file};
    use std::path::PathBuf;

    fn tree() -> Arc<FavoritesTree> {
        let src = "favorites:\n  ACME:\n    _color: \"#E4572E\"\n    Website: [Development, Code Review]\n  Break:\n    _kind: status\n";
        let (t, e) = merge_parsed(&[(PathBuf::from("a.yaml"), parse_favorites_file(src).unwrap())]);
        assert!(e.is_empty());
        Arc::new(t)
    }

    fn engine() -> Engine {
        Engine::new(
            Store::open_in_memory().unwrap(),
            EngineSettings {
                bucket: "b".into(),
                heartbeat_interval: Duration::from_secs(60),
            },
            tree(),
            t("08:00"),
        )
        .unwrap()
    }

    fn t(hm: &str) -> DateTime<Utc> {
        format!("2026-10-09T{hm}:00Z").parse().unwrap()
    }

    fn fav(id: &str) -> Target {
        Target::Favorite(id.into())
    }

    fn drain(e: &Engine) -> Vec<Op> {
        let mut out = Vec::new();
        while let Some((id, op)) = e.store.peek().unwrap() {
            e.store.remove(id).unwrap();
            out.push(op);
        }
        out
    }

    fn hb(op: &Op) -> (DateTime<Utc>, f64, String, Option<String>) {
        match op {
            Op::Heartbeat {
                timestamp,
                pulsetime,
                data,
                ..
            } => (*timestamp, *pulsetime, data.id.clone(), data.note.clone()),
            _ => panic!("expected heartbeat, got {op:?}"),
        }
    }

    #[test]
    fn start_stop_heartbeats() {
        let mut e = engine();
        assert_eq!(
            e.start(
                &fav("acme/website/development"),
                None,
                Source::Cli,
                None,
                t("09:00")
            )
            .unwrap(),
            Change::Started
        );
        let ops = drain(&e);
        assert_eq!(ops.len(), 1);
        assert_eq!(hb(&ops[0]).1, 0.0, "first heartbeat must not merge");
        assert!(!e.tick(t("09:00")).unwrap());
        assert!(e.tick(t("09:01")).unwrap());
        let ops = drain(&e);
        assert_eq!(hb(&ops[0]).1, 60.0 + 90.0);
        assert_eq!(e.stop(t("09:30")).unwrap(), Change::Stopped);
        let ops = drain(&e);
        assert_eq!(
            hb(&ops[0]),
            (
                t("09:30"),
                1800.0 + 90.0,
                "acme/website/development".into(),
                None
            )
        );
        assert!(e.running().is_none());
        assert_eq!(e.last_stopped().unwrap().ended, t("09:30"));
        assert_eq!(e.stop(t("09:31")).unwrap_err().code, ErrorCode::NotRunning);
    }

    #[test]
    fn switching_ends_previous() {
        let mut e = engine();
        e.start(
            &fav("acme/website/development"),
            None,
            Source::Cli,
            None,
            t("09:00"),
        )
        .unwrap();
        drain(&e);
        assert_eq!(
            e.start(&fav("break"), None, Source::Hotkey, None, t("10:00"))
                .unwrap(),
            Change::Switched
        );
        let ops = drain(&e);
        assert_eq!(ops.len(), 2);
        assert_eq!(hb(&ops[0]).0, t("10:00"));
        assert_eq!(hb(&ops[0]).2, "acme/website/development");
        assert_eq!(hb(&ops[1]), (t("10:00"), 0.0, "break".into(), None));
        assert_eq!(
            e.last_stopped().unwrap().activity.id,
            "acme/website/development"
        );
        let Op::Heartbeat { data, .. } = &ops[1] else {
            panic!()
        };
        assert_eq!(data.kind, Kind::Status);
        assert_eq!(data.source, Source::Hotkey);
    }

    #[test]
    fn same_activity_is_not_restarted() {
        let mut e = engine();
        e.start(&fav("break"), Some("x"), Source::Cli, None, t("09:00"))
            .unwrap();
        drain(&e);
        assert_eq!(
            e.start(&fav("break"), None, Source::Cli, None, t("09:10"))
                .unwrap(),
            Change::Unchanged
        );
        assert_eq!(
            e.start(&fav("break"), Some("x"), Source::Cli, None, t("09:10"))
                .unwrap(),
            Change::Unchanged
        );
        assert!(drain(&e).is_empty());
        assert_eq!(
            e.start(&fav("break"), Some("y"), Source::Cli, None, t("09:20"))
                .unwrap(),
            Change::NoteChanged
        );
        assert_eq!(drain(&e).len(), 2);
    }

    #[test]
    fn note_split() {
        let mut e = engine();
        e.start(
            &fav("acme/website/development"),
            Some("a"),
            Source::Cli,
            None,
            t("09:00"),
        )
        .unwrap();
        drain(&e);
        assert_eq!(
            e.set_note(Some("b"), Source::Popover, t("09:45")).unwrap(),
            Change::NoteChanged
        );
        let ops = drain(&e);
        assert_eq!(
            hb(&ops[0]),
            (
                t("09:45"),
                2700.0 + 90.0,
                "acme/website/development".into(),
                Some("a".into())
            )
        );
        assert_eq!(
            hb(&ops[1]),
            (
                t("09:45"),
                0.0,
                "acme/website/development".into(),
                Some("b".into())
            )
        );
        assert_eq!(e.running().unwrap().start, t("09:45"));
        // A note split doesn't change the "last stopped" activity.
        assert!(e.last_stopped().is_none());
        assert_eq!(
            e.set_note(Some("b"), Source::Cli, t("09:50")).unwrap(),
            Change::Unchanged
        );
        assert_eq!(
            e.set_note(None, Source::Cli, t("09:55")).unwrap(),
            Change::NoteChanged
        );
        assert_eq!(e.running().unwrap().note, None);
        e.stop(t("10:00")).unwrap();
        assert_eq!(
            e.set_note(Some("x"), Source::Cli, t("10:01"))
                .unwrap_err()
                .code,
            ErrorCode::NotRunning
        );
    }

    #[test]
    fn folders_and_unknown_ids_rejected() {
        let mut e = engine();
        assert_eq!(
            e.start(&fav("acme"), None, Source::Cli, None, t("09:00"))
                .unwrap_err()
                .code,
            ErrorCode::IsFolder
        );
        assert_eq!(
            e.start(&fav("nope"), None, Source::Cli, None, t("09:00"))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            e.log(
                &fav("acme/website"),
                t("08:00"),
                t("09:00"),
                None,
                Source::Cli,
                None,
                t("10:00")
            )
            .unwrap_err()
            .code,
            ErrorCode::IsFolder
        );
        assert!(drain(&e).is_empty());
    }

    #[test]
    fn toggle_and_resume() {
        let mut e = engine();
        assert_eq!(
            e.resume(Source::Cli, None, t("09:00")).unwrap_err().code,
            ErrorCode::NothingToResume
        );
        assert_eq!(
            e.toggle(&fav("break"), None, Source::Cli, None, t("09:00"))
                .unwrap(),
            Change::Started
        );
        assert_eq!(
            e.toggle(&fav("break"), None, Source::Cli, None, t("09:15"))
                .unwrap(),
            Change::Stopped
        );
        e.start(
            &fav("acme/website/development"),
            Some("n"),
            Source::Cli,
            None,
            t("09:20"),
        )
        .unwrap();
        e.start(&fav("break"), None, Source::Cli, None, t("09:30"))
            .unwrap();
        drain(&e);
        // Resume switches back to the previously stopped activity, with its note.
        assert_eq!(
            e.resume(Source::Hotkey, None, t("09:40")).unwrap(),
            Change::Switched
        );
        let r = e.running().unwrap();
        assert_eq!(r.activity.id, "acme/website/development");
        assert_eq!(r.note.as_deref(), Some("n"));
        assert_eq!(r.source, Source::Hotkey);
        assert_eq!(e.last_stopped().unwrap().activity.id, "break");
    }

    #[test]
    fn adhoc() {
        let mut e = engine();
        let a = Target::Adhoc(AdhocActivity {
            id: "x/y".into(),
            title: "Y".into(),
            path: vec!["X".into()],
            kind: Kind::Activity,
            origin: Origin::Adhoc,
            color: None,
            icon: None,
        });
        e.start(&a, None, Source::Cli, None, t("09:00")).unwrap();
        let ops = drain(&e);
        let Op::Heartbeat { data, .. } = &ops[0] else {
            panic!()
        };
        assert_eq!(data.origin, Origin::Adhoc);
        assert_eq!(data.path, ["X", "Y"]);
        let bad = Target::Adhoc(AdhocActivity {
            id: " ".into(),
            title: "Y".into(),
            path: vec![],
            kind: Kind::Activity,
            origin: Origin::Adhoc,
            color: None,
            icon: None,
        });
        assert_eq!(
            e.start(&bad, None, Source::Cli, None, t("09:00"))
                .unwrap_err()
                .code,
            ErrorCode::Invalid
        );
    }

    #[test]
    fn retro_entries() {
        let mut e = engine();
        // Not running: plain retro.
        e.log(
            &fav("break"),
            t("08:00"),
            t("08:30"),
            Some("lunch"),
            Source::Cli,
            None,
            t("09:00"),
        )
        .unwrap();
        let ops = drain(&e);
        let Op::Retro {
            start,
            end,
            data,
            restart,
            ..
        } = &ops[0]
        else {
            panic!()
        };
        assert_eq!((*start, *end), (t("08:00"), t("08:30")));
        assert_eq!(data.entry, Entry::Retro);
        assert_eq!(data.note.as_deref(), Some("lunch"));
        assert!(restart.is_none());

        // Running since 09:00; an entry before the run doesn't touch it.
        e.start(
            &fav("acme/website/development"),
            None,
            Source::Cli,
            None,
            t("09:00"),
        )
        .unwrap();
        drain(&e);
        e.log(
            &fav("break"),
            t("08:30"),
            t("09:00"),
            None,
            Source::Cli,
            None,
            t("10:00"),
        )
        .unwrap();
        let ops = drain(&e);
        let Op::Retro { restart, .. } = &ops[0] else {
            panic!()
        };
        assert!(restart.is_none());

        // Overlapping the run: the run restarts at the end of the entry.
        e.log(
            &fav("break"),
            t("09:30"),
            t("09:45"),
            None,
            Source::Cli,
            None,
            t("10:00"),
        )
        .unwrap();
        let ops = drain(&e);
        let Op::Retro {
            restart: Some(r), ..
        } = &ops[0]
        else {
            panic!("{ops:?}")
        };
        assert_eq!(r.run_start, t("09:00"));
        assert_eq!(r.data.id, "acme/website/development");
        assert_eq!(e.running().unwrap().start, t("09:45"));
        // Heartbeat pulsetime is now relative to the new start.
        assert!(e.tick(t("10:00")).unwrap());
        let ops = drain(&e);
        assert_eq!(hb(&ops[0]).1, 900.0 + 90.0);

        // Validation.
        assert_eq!(
            e.log(
                &fav("break"),
                t("09:00"),
                t("11:00"),
                None,
                Source::Cli,
                None,
                t("10:00")
            )
            .unwrap_err()
            .code,
            ErrorCode::Invalid
        );
        assert_eq!(
            e.log(
                &fav("break"),
                t("09:00"),
                t("09:00"),
                None,
                Source::Cli,
                None,
                t("10:00")
            )
            .unwrap_err()
            .code,
            ErrorCode::Invalid
        );
    }

    #[test]
    fn restart_after_crash_is_not_resumed() {
        let store = Store::open_in_memory().unwrap();
        let settings = EngineSettings {
            bucket: "b".into(),
            heartbeat_interval: Duration::from_secs(60),
        };
        {
            let mut e = Engine::new(store.clone(), settings.clone(), tree(), t("08:00")).unwrap();
            e.start(&fav("break"), Some("n"), Source::Cli, None, t("09:00"))
                .unwrap();
            e.tick(t("09:05")).unwrap();
            // Simulated crash: no stop.
        }
        let pending_before = store.pending().unwrap();
        let mut e = Engine::new(store.clone(), settings, tree(), t("12:00")).unwrap();
        assert!(e.running().is_none());
        let restored = e.take_restored().unwrap();
        assert_eq!(restored.ended, t("09:05"));
        assert_eq!(restored.note.as_deref(), Some("n"));
        assert!(e.take_restored().is_none());
        // No final heartbeat is written: the activity ends with its last heartbeat.
        assert_eq!(store.pending().unwrap(), pending_before);
        assert_eq!(
            e.resume(Source::Cli, None, t("12:01")).unwrap(),
            Change::Started
        );
        assert_eq!(e.running().unwrap().start, t("12:01"));
    }

    #[test]
    fn bucket_change_moves_running_activity() {
        let mut e = engine();
        e.start(&fav("break"), None, Source::Cli, None, t("09:00"))
            .unwrap();
        drain(&e);
        e.apply_settings(
            EngineSettings {
                bucket: "new".into(),
                heartbeat_interval: Duration::from_secs(60),
            },
            t("09:10"),
        )
        .unwrap();
        let ops = drain(&e);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].bucket(), "b");
        assert_eq!(ops[1].bucket(), "new");
        assert_eq!(hb(&ops[1]).1, 0.0);
        assert_eq!(e.running().unwrap().start, t("09:10"));
        // Same bucket: nothing happens.
        e.apply_settings(
            EngineSettings {
                bucket: "new".into(),
                heartbeat_interval: Duration::from_secs(30),
            },
            t("09:20"),
        )
        .unwrap();
        assert!(drain(&e).is_empty());
    }

    #[test]
    fn resume_uses_snapshot_for_deleted_favorite() {
        let mut e = engine();
        e.start(&fav("break"), None, Source::Cli, None, t("09:00"))
            .unwrap();
        e.stop(t("09:10")).unwrap();
        e.set_favorites(Arc::new(FavoritesTree::default()));
        assert_eq!(
            e.resume(Source::Cli, None, t("09:20")).unwrap(),
            Change::Started
        );
        assert_eq!(e.running().unwrap().activity.title, "Break");
    }
}
