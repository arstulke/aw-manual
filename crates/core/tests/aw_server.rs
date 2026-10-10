//! Integration tests against real aw-server instances.
//!
//! Set `AW_TEST_SERVERS` to a comma-separated list of server URLs (for
//! example aw-server-rust and aw-server-python started with `--testing`).
//! Without it the tests are skipped.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aw_manual_core::aw::{AwClient, AwEvent};
use aw_manual_core::engine::{Engine, EngineSettings, Target};
use aw_manual_core::favorites::{merge_parsed, parse_favorites_file, FavoritesTree};
use aw_manual_core::model::Source;
use aw_manual_core::store::Store;
use aw_manual_core::writer::{ServerTarget, Writer};
use chrono::{DateTime, Utc};

fn servers() -> Vec<String> {
    std::env::var("AW_TEST_SERVERS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn tree() -> Arc<FavoritesTree> {
    let src = "favorites:\n  ACME:\n    Website: [Development, Code Review]\n  Break:\n    _kind: status\n";
    Arc::new(merge_parsed(&[(PathBuf::from("a.yaml"), parse_favorites_file(src).unwrap())]).0)
}

fn t(hms: &str) -> DateTime<Utc> {
    format!("2026-01-05T{hms}Z").parse().unwrap()
}

fn fav(id: &str) -> Target {
    Target::Favorite(id.into())
}

struct Harness {
    engine: Engine,
    store: Store,
    writer: Writer,
    client: AwClient,
    bucket: String,
    url: String,
}

impl Harness {
    async fn new(url: &str, name: &str) -> Harness {
        let bucket = format!("aw-manual-test_{name}_{}", std::process::id());
        let http = reqwest::Client::new();
        let _ = http
            .delete(format!("{url}/api/0/buckets/{bucket}?force=1"))
            .send()
            .await;
        let store = Store::open_in_memory().unwrap();
        let settings = EngineSettings {
            bucket: bucket.clone(),
            heartbeat_interval: Duration::from_secs(60),
        };
        let engine = Engine::new(store.clone(), settings, tree(), t("00:00:00")).unwrap();
        let writer = Writer::new(
            store.clone(),
            ServerTarget {
                url: url.into(),
                bucket_type: "manual.activity".into(),
                bucket: bucket.clone(),
            },
            "test-host".into(),
        );
        Harness {
            engine,
            store,
            writer,
            client: AwClient::new(url),
            bucket,
            url: url.into(),
        }
    }

    /// Applies all queued operations in order.
    async fn flush(&self) {
        while let Some((id, op)) = self.store.peek().unwrap() {
            self.writer
                .apply(&op)
                .await
                .unwrap_or_else(|e| panic!("{}: {e} ({op:?})", self.url));
            self.store.remove(id).unwrap();
        }
    }

    /// Like flush, but applies each operation twice (simulated replay after a crash).
    async fn flush_twice(&self) {
        while let Some((id, op)) = self.store.peek().unwrap() {
            self.writer.apply(&op).await.unwrap();
            self.writer.apply(&op).await.unwrap();
            self.store.remove(id).unwrap();
        }
    }

    async fn events(&self) -> Vec<(String, String, String, String)> {
        let mut evs: Vec<AwEvent> = self
            .client
            .events_between(&self.bucket, t("00:00:00"), t("23:59:59"))
            .await
            .unwrap();
        // Full extents (some servers clip to the range, which is large enough here).
        evs.sort_by_key(|e| e.timestamp);
        let out: Vec<_> = evs
            .iter()
            .map(|e| {
                (
                    e.timestamp.format("%H:%M:%S").to_string(),
                    e.end().format("%H:%M:%S").to_string(),
                    e.data["id"].as_str().unwrap_or_default().to_string(),
                    format!(
                        "{}{}",
                        e.data["entry"].as_str().unwrap_or_default(),
                        e.data["note"]
                            .as_str()
                            .map(|n| format!(":{n}"))
                            .unwrap_or_default()
                    ),
                )
            })
            .collect();
        // Events must never overlap.
        for w in evs.windows(2) {
            assert!(
                w[0].end() <= w[1].timestamp,
                "{}: overlapping events: {out:?}",
                self.url
            );
        }
        out
    }

    async fn cleanup(&self) {
        let http = reqwest::Client::new();
        let _ = http
            .delete(format!(
                "{}/api/0/buckets/{}?force=1",
                self.url, self.bucket
            ))
            .send()
            .await;
    }
}

fn ev(a: &str, b: &str, id: &str, entry: &str) -> (String, String, String, String) {
    (a.into(), b.into(), id.into(), entry.into())
}

const DEV: &str = "acme/website/development";
const CR: &str = "acme/website/code-review";

#[tokio::test]
async fn timer_lifecycle() {
    for url in servers() {
        let mut h = Harness::new(&url, "lifecycle").await;
        let e = &mut h.engine;
        e.start(&fav(DEV), None, Source::Cli, None, t("09:00:00"))
            .unwrap();
        e.tick(t("09:01:00")).unwrap();
        e.tick(t("09:02:00")).unwrap();
        // A suspend: no heartbeats for two hours, the activity keeps running.
        e.tick(t("11:02:00")).unwrap();
        e.stop(t("11:02:30")).unwrap();
        h.flush().await;
        assert_eq!(
            h.events().await,
            vec![ev("09:00:00", "11:02:30", DEV, "timer")],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn resume_after_short_gap_is_a_new_event() {
    for url in servers() {
        let mut h = Harness::new(&url, "gap").await;
        let e = &mut h.engine;
        e.start(&fav(DEV), None, Source::Cli, None, t("09:00:00"))
            .unwrap();
        e.stop(t("09:10:00")).unwrap();
        e.resume(Source::Cli, None, t("09:10:30")).unwrap();
        e.tick(t("09:11:30")).unwrap();
        e.stop(t("09:12:00")).unwrap();
        h.flush().await;
        assert_eq!(
            h.events().await,
            vec![
                ev("09:00:00", "09:10:00", DEV, "timer"),
                ev("09:10:30", "09:12:00", DEV, "timer")
            ],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn switch_and_note_split() {
    for url in servers() {
        let mut h = Harness::new(&url, "switch").await;
        let e = &mut h.engine;
        e.start(&fav(DEV), Some("a"), Source::Cli, None, t("09:00:00"))
            .unwrap();
        e.tick(t("09:01:00")).unwrap();
        e.set_note(Some("b"), Source::Cli, t("09:01:30")).unwrap();
        e.tick(t("09:02:30")).unwrap();
        e.start(&fav("break"), None, Source::Cli, None, t("09:03:00"))
            .unwrap();
        e.stop(t("09:04:00")).unwrap();
        h.flush().await;
        assert_eq!(
            h.events().await,
            vec![
                ev("09:00:00", "09:01:30", DEV, "timer:a"),
                ev("09:01:30", "09:03:00", DEV, "timer:b"),
                ev("09:03:00", "09:04:00", "break", "timer"),
            ],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn retro_before_running_activity() {
    for url in servers() {
        let mut h = Harness::new(&url, "retro-before").await;
        let e = &mut h.engine;
        e.start(&fav(DEV), None, Source::Cli, None, t("09:00:00"))
            .unwrap();
        e.tick(t("09:01:00")).unwrap();
        e.log(
            &fav(CR),
            t("08:00:00"),
            t("08:30:00"),
            None,
            Source::Cli,
            None,
            t("09:01:30"),
        )
        .unwrap();
        e.tick(t("09:02:00")).unwrap();
        e.stop(t("09:03:00")).unwrap();
        h.flush().await;
        assert_eq!(
            h.events().await,
            vec![
                ev("08:00:00", "08:30:00", CR, "retro"),
                ev("09:00:00", "09:03:00", DEV, "timer")
            ],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn retro_inside_running_activity_restarts_it() {
    for url in servers() {
        let mut h = Harness::new(&url, "retro-inside").await;
        let e = &mut h.engine;
        e.start(&fav(DEV), None, Source::Cli, None, t("09:00:00"))
            .unwrap();
        for m in 1..=50 {
            e.tick(t(&format!("09:{m:02}:00"))).unwrap();
        }
        e.log(
            &fav(CR),
            t("09:10:00"),
            t("09:20:00"),
            None,
            Source::Cli,
            None,
            t("09:50:30"),
        )
        .unwrap();
        e.tick(t("09:51:00")).unwrap();
        e.tick(t("09:52:00")).unwrap();
        e.stop(t("09:52:30")).unwrap();
        h.flush().await;
        assert_eq!(
            h.events().await,
            vec![
                ev("09:00:00", "09:10:00", DEV, "timer"),
                ev("09:10:00", "09:20:00", CR, "retro"),
                ev("09:20:00", "09:52:30", DEV, "timer"),
            ],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn retro_until_now_while_running() {
    for url in servers() {
        let mut h = Harness::new(&url, "retro-now").await;
        let e = &mut h.engine;
        e.start(&fav(DEV), None, Source::Cli, None, t("09:00:00"))
            .unwrap();
        e.tick(t("09:01:00")).unwrap();
        e.tick(t("09:02:00")).unwrap();
        // "-30s" at 09:02:40: covers the time since the last heartbeat.
        e.log(
            &fav(CR),
            t("09:02:10"),
            t("09:02:40"),
            None,
            Source::Cli,
            None,
            t("09:02:40"),
        )
        .unwrap();
        e.tick(t("09:03:40")).unwrap();
        e.stop(t("09:04:00")).unwrap();
        h.flush().await;
        assert_eq!(
            h.events().await,
            vec![
                ev("09:00:00", "09:02:10", DEV, "timer"),
                ev("09:02:10", "09:02:40", CR, "retro"),
                ev("09:02:40", "09:04:00", DEV, "timer"),
            ],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn retro_covering_start_of_running_activity() {
    for url in servers() {
        let mut h = Harness::new(&url, "retro-cover").await;
        let e = &mut h.engine;
        e.start(&fav(DEV), None, Source::Cli, None, t("09:00:00"))
            .unwrap();
        e.tick(t("09:01:00")).unwrap();
        e.tick(t("09:02:00")).unwrap();
        e.log(
            &fav(CR),
            t("08:50:00"),
            t("09:01:30"),
            None,
            Source::Cli,
            None,
            t("09:02:30"),
        )
        .unwrap();
        e.stop(t("09:03:00")).unwrap();
        h.flush().await;
        assert_eq!(
            h.events().await,
            vec![
                ev("08:50:00", "09:01:30", CR, "retro"),
                ev("09:01:30", "09:03:00", DEV, "timer")
            ],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn retro_splits_and_trims_past_events() {
    for url in servers() {
        let mut h = Harness::new(&url, "retro-split").await;
        let e = &mut h.engine;
        e.log(
            &fav(DEV),
            t("08:00:00"),
            t("10:00:00"),
            None,
            Source::Cli,
            None,
            t("12:00:00"),
        )
        .unwrap();
        e.log(
            &fav("break"),
            t("10:00:00"),
            t("10:30:00"),
            None,
            Source::Cli,
            None,
            t("12:00:00"),
        )
        .unwrap();
        // Splits the first entry.
        e.log(
            &fav(CR),
            t("08:30:00"),
            t("09:00:00"),
            None,
            Source::Cli,
            None,
            t("12:00:00"),
        )
        .unwrap();
        // Trims the end of the first and the start of the break.
        e.log(
            &fav(CR),
            t("09:45:00"),
            t("10:15:00"),
            Some("x"),
            Source::Cli,
            None,
            t("12:00:00"),
        )
        .unwrap();
        // Replaces an entry completely.
        e.log(
            &fav(DEV),
            t("08:29:00"),
            t("09:01:00"),
            Some("y"),
            Source::Cli,
            None,
            t("12:00:00"),
        )
        .unwrap();
        h.flush_twice().await;
        assert_eq!(
            h.events().await,
            vec![
                ev("08:00:00", "08:29:00", DEV, "retro"),
                ev("08:29:00", "09:01:00", DEV, "retro:y"),
                ev("09:01:00", "09:45:00", DEV, "retro"),
                ev("09:45:00", "10:15:00", CR, "retro:x"),
                ev("10:15:00", "10:30:00", "break", "retro"),
            ],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn adjacent_identical_retro_entries_while_running() {
    for url in servers() {
        let mut h = Harness::new(&url, "retro-adjacent").await;
        let e = &mut h.engine;
        e.start(&fav(DEV), None, Source::Cli, None, t("09:00:00"))
            .unwrap();
        e.tick(t("09:30:00")).unwrap();
        e.log(
            &fav(CR),
            t("09:05:00"),
            t("09:10:00"),
            None,
            Source::Cli,
            None,
            t("09:30:10"),
        )
        .unwrap();
        e.log(
            &fav(CR),
            t("09:10:00"),
            t("09:15:00"),
            None,
            Source::Cli,
            None,
            t("09:30:20"),
        )
        .unwrap();
        e.tick(t("09:31:20")).unwrap();
        e.stop(t("09:32:00")).unwrap();
        h.flush_twice().await;
        assert_eq!(
            h.events().await,
            vec![
                ev("09:00:00", "09:05:00", DEV, "timer"),
                ev("09:05:00", "09:10:00", CR, "retro"),
                ev("09:10:00", "09:15:00", CR, "retro"),
                ev("09:15:00", "09:32:00", DEV, "timer"),
            ],
            "{url}"
        );
        h.cleanup().await;
    }
}

#[tokio::test]
async fn offline_buffer_replays_in_order() {
    for url in servers() {
        let store = Store::open_in_memory().unwrap();
        let bucket = format!("aw-manual-test_offline_{}", std::process::id());
        let settings = EngineSettings {
            bucket: bucket.clone(),
            heartbeat_interval: Duration::from_secs(60),
        };
        let mut engine = Engine::new(store.clone(), settings, tree(), t("00:00:00")).unwrap();
        // The writer points to an unreachable server first.
        let writer = Writer::new(
            store.clone(),
            ServerTarget {
                url: "http://127.0.0.1:9".into(),
                bucket_type: "manual.activity".into(),
                bucket: bucket.clone(),
            },
            "test-host".into(),
        );
        engine.set_queue_listener({
            let w = writer.clone();
            move || w.notify()
        });
        let mut status = writer.subscribe();
        let task = tokio::spawn(writer.clone().run());
        engine
            .start(&fav(DEV), None, Source::Cli, None, t("09:00:00"))
            .unwrap();
        engine.tick(t("09:01:00")).unwrap();
        engine.stop(t("09:02:00")).unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                status.changed().await.unwrap();
                let s = status.borrow().clone();
                if s.online == Some(false) && s.pending == 3 {
                    break;
                }
            }
        })
        .await
        .expect("writer reports offline with 3 pending entries");
        // The server comes back.
        writer
            .set_target(ServerTarget {
                url: url.clone(),
                bucket_type: "manual.activity".into(),
                bucket: bucket.clone(),
            })
            .await;
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if store.pending().unwrap() == 0 && writer.status().online == Some(true) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("queue drained");
        task.abort();
        let client = AwClient::new(&url);
        let evs = client
            .events_between(&bucket, t("00:00:00"), t("23:59:59"))
            .await
            .unwrap();
        assert_eq!(evs.len(), 1, "{url}");
        assert_eq!(evs[0].timestamp, t("09:00:00"));
        assert_eq!(evs[0].end(), t("09:02:00"));
        let _ = reqwest::Client::new()
            .delete(format!("{url}/api/0/buckets/{bucket}?force=1"))
            .send()
            .await;
    }
}
