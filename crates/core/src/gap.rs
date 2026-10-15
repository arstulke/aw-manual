//! Time the user couldn't track (aw-manual not running after a restart, or
//! the screen locked), for the popover's "What were you doing?" prompt.
//!
//! aw-manual only asks: nothing is recorded unless the user picks an activity.

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::Serialize;

use crate::model::Activity;

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
    /// The gap as a retroactive time range (`2026-10-09 10:03-10:45`, local
    /// time), `None` if it is shorter than a minute (nothing to log then).
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
        let minutes = (end - start).num_minutes().max(0);
        Gap {
            reason,
            start,
            end,
            minutes,
            range: (minutes >= 1).then(|| range_text(start, end, &Local)),
            last,
        }
    }
}

/// Formats a period as a range the time parser understands, in local time.
/// Minutes are truncated, so the range never ends in the future.
pub fn range_text<Tz: TimeZone>(start: DateTime<Utc>, end: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let (s, e) = (start.with_timezone(tz), end.with_timezone(tz));
    format!("{}-{}", s.format("%Y-%m-%d %H:%M"), e.format("%H:%M"))
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
        assert_eq!(r, "2026-10-09 10:03-10:45");
        let now = DateTime::parse_from_rfc3339("2026-10-09T10:46:00+02:00").unwrap();
        let parsed = crate::timeparse::parse_range(&r, &now).unwrap();
        assert_eq!(
            parsed,
            (ts("2026-10-09T08:03:00Z"), ts("2026-10-09T08:45:00Z"))
        );
    }

    #[test]
    fn across_midnight() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let r = range_text(ts("2026-10-08T23:50:00Z"), ts("2026-10-09T00:20:00Z"), &tz);
        assert_eq!(r, "2026-10-08 23:50-00:20");
        let now = DateTime::parse_from_rfc3339("2026-10-09T00:21:00+00:00").unwrap();
        let parsed = crate::timeparse::parse_range(&r, &now).unwrap();
        assert_eq!(
            parsed,
            (ts("2026-10-08T23:50:00Z"), ts("2026-10-09T00:20:00Z"))
        );
    }

    #[test]
    fn short_gaps_have_no_range() {
        let g = Gap::new(
            GapReason::Restart,
            ts("2026-10-09T08:00:00Z"),
            ts("2026-10-09T08:00:40Z"),
            None,
        );
        assert_eq!((g.minutes, g.range), (0, None));
        let g = Gap::new(
            GapReason::Unlock,
            ts("2026-10-09T08:00:00Z"),
            ts("2026-10-09T08:42:10Z"),
            None,
        );
        assert_eq!(g.minutes, 42);
        assert!(g.range.is_some());
    }
}
