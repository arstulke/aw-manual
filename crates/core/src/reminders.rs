//! Decides when reminder notifications are due (pure logic; the daemon shows them).

use chrono::{DateTime, Datelike, TimeZone, Utc};

use crate::config::{NotificationsConfig, WorkHours};
use crate::model::Run;
use crate::writer::WriterStatus;

#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// No activity has been running for `minutes`.
    Idle { minutes: i64 },
    /// The same activity has been running for a long time.
    LongRunning {
        title: String,
        path: Vec<String>,
        minutes: i64,
    },
    /// aw-server is unreachable; entries are buffered.
    Offline { pending: usize },
}

#[derive(Debug, Clone, Default)]
pub struct ReminderState {
    idle_notified_for: Option<DateTime<Utc>>,
    offline_notified: bool,
}

/// Start of today's working hours, if `now` lies within them.
fn work_start<Tz: TimeZone>(wh: &WorkHours, now: &DateTime<Tz>) -> Option<DateTime<Utc>> {
    if !wh.days.contains(&now.weekday()) {
        return None;
    }
    let t = now.time();
    if t < wh.from || t >= wh.to {
        return None;
    }
    now.timezone()
        .from_local_datetime(&now.date_naive().and_time(wh.from))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
}

/// Returns the notices that are due now. `long_running_notified` reports
/// whether the long-running notice was already shown for the current run.
pub fn check<Tz: TimeZone>(
    cfg: &NotificationsConfig,
    running: Option<&Run>,
    idle_since: DateTime<Utc>,
    writer: &WriterStatus,
    now: &DateTime<Tz>,
    state: &mut ReminderState,
) -> Vec<Notice> {
    let now_utc = now.with_timezone(&Utc);
    let mut out = Vec::new();

    match running {
        None => {
            if let (Some(limit), Some(ws)) = (cfg.idle_reminder, work_start(&cfg.work_hours, now)) {
                let since = idle_since.max(ws);
                let idle = now_utc - since;
                if idle >= chrono::Duration::from_std(limit).unwrap_or(chrono::Duration::MAX)
                    && state.idle_notified_for != Some(since)
                {
                    state.idle_notified_for = Some(since);
                    out.push(Notice::Idle {
                        minutes: idle.num_minutes(),
                    });
                }
            }
        }
        Some(run) => {
            if let Some(limit) = cfg.long_running {
                let elapsed = now_utc - run.start;
                if !run.long_running_notified
                    && elapsed >= chrono::Duration::from_std(limit).unwrap_or(chrono::Duration::MAX)
                {
                    out.push(Notice::LongRunning {
                        title: run.activity.title.clone(),
                        path: run.activity.path.clone(),
                        minutes: elapsed.num_minutes(),
                    });
                }
            }
        }
    }

    match writer.online {
        Some(true) => state.offline_notified = false,
        Some(false) if writer.pending > 0 && cfg.offline && !state.offline_notified => {
            state.offline_notified = true;
            out.push(Notice::Offline {
                pending: writer.pending,
            });
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::favorites::Kind;
    use crate::model::{Activity, Origin, Source};
    use chrono::FixedOffset;

    fn local(s: &str) -> DateTime<FixedOffset> {
        // 2026-10-09 is a Friday.
        DateTime::parse_from_rfc3339(&format!("2026-10-{s}:00+02:00")).unwrap()
    }

    fn run(start: DateTime<Utc>) -> Run {
        Run {
            activity: Activity {
                id: "a".into(),
                title: "A".into(),
                path: vec!["A".into()],
                kind: Kind::Activity,
                origin: Origin::Config,
                color: None,
                icon: None,
            },
            note: None,
            source: Source::Cli,
            device: None,
            start,
            last_heartbeat: start,
            long_running_notified: false,
        }
    }

    #[test]
    fn idle_reminder_within_work_hours_once() {
        let cfg = Config::default().notifications;
        let w = WriterStatus::default();
        let mut st = ReminderState::default();
        let idle_since = local("09T07:00").with_timezone(&Utc);
        // Before work hours: nothing.
        assert!(check(&cfg, None, idle_since, &w, &local("09T07:59"), &mut st).is_empty());
        // Idle counts from the start of work hours.
        assert!(check(&cfg, None, idle_since, &w, &local("09T08:14"), &mut st).is_empty());
        assert_eq!(
            check(&cfg, None, idle_since, &w, &local("09T08:15"), &mut st),
            vec![Notice::Idle { minutes: 15 }]
        );
        // Only once per idle period.
        assert!(check(&cfg, None, idle_since, &w, &local("09T09:00"), &mut st).is_empty());
        // A new idle period (activity stopped later) triggers again.
        let later = local("09T10:00").with_timezone(&Utc);
        assert!(check(&cfg, None, later, &w, &local("09T10:10"), &mut st).is_empty());
        assert_eq!(
            check(&cfg, None, later, &w, &local("09T10:15"), &mut st).len(),
            1
        );
        // Weekend (Saturday): nothing.
        let mut st = ReminderState::default();
        assert!(check(&cfg, None, idle_since, &w, &local("10T12:00"), &mut st).is_empty());
        // After work hours: nothing.
        assert!(check(&cfg, None, idle_since, &w, &local("09T18:30"), &mut st).is_empty());
    }

    #[test]
    fn idle_disabled() {
        let mut cfg = Config::default().notifications;
        cfg.idle_reminder = None;
        let mut st = ReminderState::default();
        let since = local("09T07:00").with_timezone(&Utc);
        assert!(check(
            &cfg,
            None,
            since,
            &WriterStatus::default(),
            &local("09T12:00"),
            &mut st
        )
        .is_empty());
    }

    #[test]
    fn long_running() {
        let cfg = Config::default().notifications;
        let mut st = ReminderState::default();
        let w = WriterStatus::default();
        let mut r = run(local("09T08:00").with_timezone(&Utc));
        let since = local("09T07:00").with_timezone(&Utc);
        assert!(check(&cfg, Some(&r), since, &w, &local("09T10:59"), &mut st).is_empty());
        let n = check(&cfg, Some(&r), since, &w, &local("09T11:00"), &mut st);
        assert_eq!(
            n,
            vec![Notice::LongRunning {
                title: "A".into(),
                path: vec!["A".into()],
                minutes: 180
            }]
        );
        r.long_running_notified = true;
        assert!(check(&cfg, Some(&r), since, &w, &local("09T12:00"), &mut st).is_empty());
        // Also outside working hours.
        let r = run(local("09T20:00").with_timezone(&Utc));
        assert_eq!(
            check(&cfg, Some(&r), since, &w, &local("09T23:30"), &mut st).len(),
            1
        );
    }

    #[test]
    fn offline_once_per_period() {
        let cfg = Config::default().notifications;
        let mut st = ReminderState::default();
        let since = local("09T07:00").with_timezone(&Utc);
        let r = run(since);
        let off = WriterStatus {
            online: Some(false),
            pending: 3,
            ..Default::default()
        };
        let now = local("09T07:30");
        assert_eq!(
            check(&cfg, Some(&r), since, &off, &now, &mut st),
            vec![Notice::Offline { pending: 3 }]
        );
        assert!(check(&cfg, Some(&r), since, &off, &now, &mut st).is_empty());
        let on = WriterStatus {
            online: Some(true),
            ..Default::default()
        };
        assert!(check(&cfg, Some(&r), since, &on, &now, &mut st).is_empty());
        assert_eq!(check(&cfg, Some(&r), since, &off, &now, &mut st).len(), 1);
        // Offline without pending entries: nothing to report.
        let mut st = ReminderState::default();
        let off0 = WriterStatus {
            online: Some(false),
            pending: 0,
            ..Default::default()
        };
        assert!(check(&cfg, Some(&r), since, &off0, &now, &mut st).is_empty());
    }
}
