//! Time the user couldn't track (aw-manual not running after a restart, or
//! the screen locked), for the popover's "What were you doing?" prompt.
//!
//! aw-manual only asks: nothing is recorded unless the user picks an activity.

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::Serialize;

use crate::model::{Activity, Stopped};

/// Why the popover asks about a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GapReason {
    /// aw-manual wasn't running (reboot, crash, quit).
    Restart,
    /// The screen was locked.
    Unlock,
}

/// The activity before the gap, offered to continue when the user skips the question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GapActivity {
    pub id: String,
    pub title: String,
    pub path: Vec<String>,
    pub note: Option<String>,
}

impl GapActivity {
    pub fn new(activity: &Activity, note: Option<&str>) -> Self {
        GapActivity {
            id: activity.id.clone(),
            title: activity.title.clone(),
            path: activity.path.clone(),
            note: note.map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gap {
    pub reason: GapReason,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Length in whole minutes.
    pub minutes: i64,
    /// Length in whole seconds.
    pub seconds: i64,
    /// The gap as a retroactive time range (`2026-10-09 10:03:27-10:45:05`,
    /// local time), `None` if it is shorter than a second (nothing to log).
    pub range: Option<String>,
    /// The activity before the gap (restart: the last one; unlock: the running one).
    pub last: Option<GapActivity>,
}

impl Gap {
    pub fn new(
        reason: GapReason,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        last: Option<GapActivity>,
    ) -> Gap {
        let seconds = (end - start).num_seconds().max(0);
        Gap {
            reason,
            start,
            end,
            minutes: seconds / 60,
            seconds,
            range: (seconds >= 1).then(|| range_text(start, end, &Local)),
            last,
        }
    }
}

/// What to ask after the daemon started.
///
/// - `previous_alive`: when the previous daemon was last known to be running.
/// - `restored`: the activity that was still running when it stopped.
///
/// If aw-manual was stopped at least `min`, the whole flow runs (log the gap,
/// then continue). If it was stopped for a shorter time but an activity was
/// running, only "continue with it?" is asked (no range). Otherwise nothing.
pub fn restart_gap(
    previous_alive: Option<DateTime<Utc>>,
    restored: Option<&Stopped>,
    now: DateTime<Utc>,
    min: std::time::Duration,
) -> Option<Gap> {
    // The restored activity ended with its last heartbeat: the gap starts
    // there, so it leaves no hole.
    let start = restored.map(|r| r.ended).or(previous_alive)?;
    let last = restored.map(|r| GapActivity::new(&r.activity, r.note.as_deref()));
    let stopped_for = (now - start).to_std().unwrap_or_default();
    let mut gap = Gap::new(GapReason::Restart, start, now, last);
    if stopped_for < min {
        gap.last.as_ref()?;
        gap.range = None;
    }
    Some(gap)
}

/// Formats a period as a range the time parser understands, in local time.
/// Fractions of seconds are truncated, so the range never ends in the future.
pub fn range_text<Tz: TimeZone>(start: DateTime<Utc>, end: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let (s, e) = (start.with_timezone(tz), end.with_timezone(tz));
    format!("{}-{}", s.format("%Y-%m-%d %H:%M:%S"), e.format("%H:%M:%S"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn ts(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn range_in_local_time_parses_back() {
        let tz = FixedOffset::east_opt(2 * 3600).unwrap();
        let (start, end) = (ts("2026-10-09T08:03:27Z"), ts("2026-10-09T08:45:50Z"));
        let r = range_text(start, end, &tz);
        assert_eq!(r, "2026-10-09 10:03:27-10:45:50");
        let now = DateTime::parse_from_rfc3339("2026-10-09T10:46:00+02:00").unwrap();
        let parsed = crate::timeparse::parse_range(&r, &now).unwrap();
        assert_eq!(
            parsed,
            (ts("2026-10-09T08:03:27Z"), ts("2026-10-09T08:45:50Z"))
        );
    }

    #[test]
    fn across_midnight() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let r = range_text(ts("2026-10-08T23:50:00Z"), ts("2026-10-09T00:20:00Z"), &tz);
        assert_eq!(r, "2026-10-08 23:50:00-00:20:00");
        let now = DateTime::parse_from_rfc3339("2026-10-09T00:21:00+00:00").unwrap();
        let parsed = crate::timeparse::parse_range(&r, &now).unwrap();
        assert_eq!(
            parsed,
            (ts("2026-10-08T23:50:00Z"), ts("2026-10-09T00:20:00Z"))
        );
    }

    fn stopped(ended: &str) -> Stopped {
        Stopped {
            activity: Activity {
                id: "acme/dev".into(),
                title: "Dev".into(),
                path: vec!["ACME".into(), "Dev".into()],
                kind: crate::favorites::Kind::Activity,
                origin: crate::model::Origin::Config,
                color: None,
                icon: None,
            },
            note: Some("n".into()),
            ended: ts(ended),
        }
    }

    #[test]
    fn restart() {
        const MIN: std::time::Duration = std::time::Duration::from_secs(60);
        let now = ts("2026-10-09T09:00:00Z");
        // First start ever: nothing to ask.
        assert_eq!(restart_gap(None, None, now, MIN), None);
        // Daemon (or computer) restarted with nothing running.
        let g = restart_gap(Some(ts("2026-10-09T08:30:00Z")), None, now, MIN).unwrap();
        assert_eq!(
            (g.start, g.minutes, g.last),
            (ts("2026-10-09T08:30:00Z"), 30, None)
        );
        assert!(g.range.is_some());
        // Restarted quickly with nothing running: nothing to ask.
        assert_eq!(
            restart_gap(Some(ts("2026-10-09T08:59:40Z")), None, now, MIN),
            None
        );
        // With a running activity: the gap starts at its last heartbeat.
        let r = stopped("2026-10-09T08:29:20Z");
        let g = restart_gap(Some(ts("2026-10-09T08:30:00Z")), Some(&r), now, MIN).unwrap();
        assert_eq!(g.start, ts("2026-10-09T08:29:20Z"));
        assert_eq!(g.last.unwrap().note.as_deref(), Some("n"));
        // Quick restart with a running activity: only "continue?".
        let r = stopped("2026-10-09T08:59:30Z");
        let g = restart_gap(Some(ts("2026-10-09T08:59:50Z")), Some(&r), now, MIN).unwrap();
        assert_eq!(g.range, None);
        assert!(g.last.is_some());
    }

    #[test]
    fn short_gaps() {
        let g = Gap::new(
            GapReason::Unlock,
            ts("2026-10-09T08:00:00Z"),
            ts("2026-10-09T08:00:00.400Z"),
            None,
        );
        assert_eq!((g.seconds, g.range), (0, None));
        let g = Gap::new(
            GapReason::Unlock,
            ts("2026-10-09T08:00:00Z"),
            ts("2026-10-09T08:00:10Z"),
            None,
        );
        assert_eq!((g.minutes, g.seconds), (0, 10));
        assert!(g.range.is_some());
        let g = Gap::new(
            GapReason::Unlock,
            ts("2026-10-09T08:00:00Z"),
            ts("2026-10-09T08:42:10Z"),
            None,
        );
        assert_eq!((g.minutes, g.seconds), (42, 2530));
    }
}
