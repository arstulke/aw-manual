//! Local SQLite storage: the offline write queue and the persisted tracking state.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::model::{EventData, Run, Stopped};

/// A write operation for aw-server, stored until it was applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// A heartbeat for the running activity.
    Heartbeat {
        bucket: String,
        timestamp: DateTime<Utc>,
        data: EventData,
        /// Seconds. The first heartbeat of a run has a duration of 1 ms, so
        /// the running event always ends last (see [`crate::writer`]).
        #[serde(default)]
        duration: f64,
        /// Seconds; 0 for the first heartbeat of a run (never merge into an earlier event).
        pulsetime: f64,
    },
    /// A retroactive entry; overlapping events in the bucket are trimmed or split.
    Retro {
        bucket: String,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        data: EventData,
        /// Set when the entry overlaps the running activity, which restarts at `end`.
        restart: Option<Restart>,
    },
}

impl Op {
    pub fn bucket(&self) -> &str {
        match self {
            Op::Heartbeat { bucket, .. } | Op::Retro { bucket, .. } => bucket,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Restart {
    /// Event data of the running activity.
    pub data: EventData,
    /// Start of the running activity's current event before the retroactive entry.
    pub run_start: DateTime<Utc>,
}

/// Persisted tracking state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedState {
    pub running: Option<Run>,
    pub last_stopped: Option<Stopped>,
}

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Store")
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    op TEXT NOT NULL,
    created TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS dead_letters (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    op TEXT NOT NULL,
    error TEXT NOT NULL,
    failed TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> anyhow::Result<Store> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push(&self, op: &Op) -> anyhow::Result<i64> {
        let json = serde_json::to_string(op)?;
        let conn = self.lock();
        conn.execute(
            "INSERT INTO queue (op, created) VALUES (?1, ?2)",
            params![json, Utc::now().to_rfc3339()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The oldest pending operation.
    pub fn peek(&self) -> anyhow::Result<Option<(i64, Op)>> {
        let conn = self.lock();
        let row: Option<(i64, String)> = conn
            .query_row("SELECT id, op FROM queue ORDER BY id LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        match row {
            None => Ok(None),
            Some((id, json)) => match serde_json::from_str(&json) {
                Ok(op) => Ok(Some((id, op))),
                Err(e) => {
                    // Unreadable entry (e.g. written by a newer version): park it.
                    conn.execute(
                        "INSERT INTO dead_letters (op, error, failed) VALUES (?1, ?2, ?3)",
                        params![json, format!("cannot decode: {e}"), Utc::now().to_rfc3339()],
                    )?;
                    conn.execute("DELETE FROM queue WHERE id = ?1", params![id])?;
                    drop(conn);
                    self.peek()
                }
            },
        }
    }

    pub fn remove(&self, id: i64) -> anyhow::Result<()> {
        self.lock()
            .execute("DELETE FROM queue WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Moves an operation that can never succeed out of the queue (kept for inspection).
    pub fn dead_letter(&self, id: i64, op: &Op, error: &str) -> anyhow::Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO dead_letters (op, error, failed) VALUES (?1, ?2, ?3)",
            params![serde_json::to_string(op)?, error, Utc::now().to_rfc3339()],
        )?;
        conn.execute("DELETE FROM queue WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn pending(&self) -> anyhow::Result<usize> {
        let n: i64 = self
            .lock()
            .query_row("SELECT COUNT(*) FROM queue", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn dead_letter_count(&self) -> anyhow::Result<usize> {
        let n: i64 = self
            .lock()
            .query_row("SELECT COUNT(*) FROM dead_letters", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn load_state(&self) -> anyhow::Result<PersistedState> {
        let v: Option<String> = self
            .lock()
            .query_row("SELECT value FROM kv WHERE key = 'state'", [], |r| r.get(0))
            .optional()?;
        Ok(match v {
            Some(s) => serde_json::from_str(&s).unwrap_or_default(),
            None => PersistedState::default(),
        })
    }

    pub fn save_state(&self, state: &PersistedState) -> anyhow::Result<()> {
        let json = serde_json::to_string(state)?;
        self.lock().execute(
            "INSERT INTO kv (key, value) VALUES ('state', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![json],
        )?;
        Ok(())
    }

    /// Atomically saves the state and queues operations.
    pub fn commit(&self, state: &PersistedState, ops: &[Op]) -> anyhow::Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO kv (key, value) VALUES ('state', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![serde_json::to_string(state)?],
        )?;
        let now = Utc::now().to_rfc3339();
        for op in ops {
            tx.execute(
                "INSERT INTO queue (op, created) VALUES (?1, ?2)",
                params![serde_json::to_string(op)?, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::favorites::Kind;
    use crate::model::{Entry, Origin, Source};

    fn data() -> EventData {
        EventData {
            id: "a".into(),
            title: "A".into(),
            path: vec!["A".into()],
            kind: Kind::Activity,
            note: None,
            source: Source::Cli,
            device: None,
            entry: Entry::Timer,
            origin: Origin::Config,
        }
    }

    #[test]
    fn queue_fifo() {
        let s = Store::open_in_memory().unwrap();
        let t = Utc::now();
        for i in 0..3 {
            s.push(&Op::Heartbeat {
                bucket: "b".into(),
                timestamp: t,
                data: data(),
                duration: 0.0,
                pulsetime: i as f64,
            })
            .unwrap();
        }
        assert_eq!(s.pending().unwrap(), 3);
        let (id, op) = s.peek().unwrap().unwrap();
        assert!(matches!(op, Op::Heartbeat { pulsetime, .. } if pulsetime == 0.0));
        s.remove(id).unwrap();
        let (id, op) = s.peek().unwrap().unwrap();
        assert!(matches!(op, Op::Heartbeat { pulsetime, .. } if pulsetime == 1.0));
        s.dead_letter(id, &op, "nope").unwrap();
        assert_eq!(s.pending().unwrap(), 1);
        assert_eq!(s.dead_letter_count().unwrap(), 1);
    }

    #[test]
    fn state_roundtrip_and_commit() {
        let dir = std::env::temp_dir().join(format!("aw-manual-store-{}", std::process::id()));
        let path = dir.join("state.db");
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.load_state().unwrap(), PersistedState::default());
        let st = PersistedState {
            running: None,
            last_stopped: Some(Stopped {
                activity: crate::model::Activity {
                    id: "a".into(),
                    title: "A".into(),
                    path: vec!["A".into()],
                    kind: Kind::Activity,
                    origin: Origin::Config,
                    color: None,
                    icon: None,
                },
                note: Some("n".into()),
                ended: Utc::now(),
            }),
        };
        s.commit(
            &st,
            &[Op::Heartbeat {
                bucket: "b".into(),
                timestamp: Utc::now(),
                data: data(),
                duration: 0.0,
                pulsetime: 0.0,
            }],
        )
        .unwrap();
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.load_state().unwrap(), st);
        assert_eq!(s.pending().unwrap(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undecodable_entries_are_parked() {
        let s = Store::open_in_memory().unwrap();
        s.lock()
            .execute(
                "INSERT INTO queue (op, created) VALUES ('{\"op\":\"future\"}', 'x')",
                [],
            )
            .unwrap();
        s.push(&Op::Heartbeat {
            bucket: "b".into(),
            timestamp: Utc::now(),
            data: data(),
            duration: 0.0,
            pulsetime: 0.0,
        })
        .unwrap();
        assert!(matches!(s.peek().unwrap(), Some((_, Op::Heartbeat { .. }))));
        assert_eq!(s.dead_letter_count().unwrap(), 1);
    }
}
