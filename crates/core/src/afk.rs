//! Detects the return from an AFK period (from aw-watcher-afk events), so the
//! popover can ask what the user was doing in the meantime.
//!
//! aw-manual only asks: nothing is recorded unless the user picks an activity.

use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use serde::Serialize;

use crate::aw::AwEvent;
use crate::favorites::Kind;
use crate::model::Run;

/// A finished AFK period the user has just returned from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AfkPeriod {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Length in whole minutes.
    pub minutes: i64,
    /// The period as a retroactive time range (`2026-10-09 10:03-10:45`, local time).
    pub range: String,
}

/// Remembers which AFK period was already asked about.
#[derive(Debug, Default)]
pub struct AfkState {
    asked_until: Option<DateTime<Utc>>,
}

/// How long after the return the question is still asked (e.g. the daemon
/// started later, or the server was unreachable).
const MAX_DELAY_MINUTES: i64 = 10;

fn status(e: &AwEvent) -> Option<&str> {
    e.data.get("status").and_then(|s| s.as_str())
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

/// Checks the latest events of the AFK bucket (in any order). Returns the AFK
/// period to ask about when the user is back (latest event `not-afk`) after
/// an AFK period of at least `threshold` that ended recently and wasn't asked
/// about yet. A status activity (break, meeting, …) that was already running
/// when the user left covers the period, so nothing is asked then.
pub fn check(
    events: &[AwEvent],
    threshold: std::time::Duration,
    running: Option<&Run>,
    now: DateTime<Utc>,
    state: &mut AfkState,
) -> Option<AfkPeriod> {
    let mut events: Vec<&AwEvent> = events.iter().collect();
    events.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    let latest = events.first()?;
    if status(latest) != Some("not-afk") {
        return None;
    }
    let afk = events
        .iter()
        .skip(1)
        .find(|e| e.end() <= latest.timestamp + Duration::seconds(1))?;
    if status(afk) != Some("afk") {
        return None;
    }
    let (start, end) = (afk.timestamp, afk.end());
    let threshold = Duration::from_std(threshold).unwrap_or(Duration::MAX);
    if end - start < threshold
        || now - end > Duration::minutes(MAX_DELAY_MINUTES)
        || state.asked_until.is_some_and(|t| end <= t)
    {
        return None;
    }
    state.asked_until = Some(end);
    if running.is_some_and(|r| r.activity.kind == Kind::Status && r.start <= start) {
        return None;
    }
    Some(AfkPeriod {
        start,
        end,
        minutes: (end - start).num_minutes(),
        range: range_text(start, end, &Local),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Activity, Origin, Source};
    use chrono::FixedOffset;
    use serde_json::json;
    use std::time::Duration as StdDuration;

    fn t(hm: &str) -> DateTime<Utc> {
        format!("2026-10-09T{hm}:00Z").parse().unwrap()
    }

    fn ev(start: &str, end: &str, status: &str) -> AwEvent {
        AwEvent {
            id: None,
            timestamp: t(start),
            duration: (t(end) - t(start)).num_seconds() as f64,
            data: json!({ "status": status }),
        }
    }

    fn run(kind: Kind, start: &str) -> Run {
        Run {
            activity: Activity {
                id: "break".into(),
                title: "Break".into(),
                path: vec!["Break".into()],
                kind,
                origin: Origin::Config,
                color: None,
                icon: None,
            },
            note: None,
            source: Source::Cli,
            device: None,
            start: t(start),
            last_heartbeat: t(start),
            long_running_notified: false,
        }
    }

    const MIN15: StdDuration = StdDuration::from_secs(15 * 60);

    #[test]
    fn asks_once_after_returning() {
        let events = vec![
            ev("10:45", "10:46", "not-afk"),
            ev("10:03", "10:45", "afk"),
            ev("09:00", "10:03", "not-afk"),
        ];
        let mut st = AfkState::default();
        let p = check(&events, MIN15, None, t("10:46"), &mut st).unwrap();
        assert_eq!((p.start, p.end, p.minutes), (t("10:03"), t("10:45"), 42));
        assert_eq!(check(&events, MIN15, None, t("10:47"), &mut st), None);
    }

    #[test]
    fn nothing_while_afk_short_or_old() {
        let mut st = AfkState::default();
        let away = vec![ev("10:03", "10:45", "afk"), ev("09:00", "10:03", "not-afk")];
        assert_eq!(check(&away, MIN15, None, t("10:46"), &mut st), None);
        let short = vec![ev("10:15", "10:16", "not-afk"), ev("10:03", "10:15", "afk")];
        assert_eq!(check(&short, MIN15, None, t("10:16"), &mut st), None);
        let back = vec![ev("10:45", "11:30", "not-afk"), ev("10:03", "10:45", "afk")];
        assert_eq!(check(&back, MIN15, None, t("11:30"), &mut st), None);
        assert_eq!(check(&[], MIN15, None, t("11:30"), &mut st), None);
    }

    #[test]
    fn running_status_covers_the_period() {
        let events = vec![ev("10:45", "10:46", "not-afk"), ev("10:03", "10:45", "afk")];
        let mut st = AfkState::default();
        let brk = run(Kind::Status, "10:00");
        assert_eq!(check(&events, MIN15, Some(&brk), t("10:46"), &mut st), None);
        let mut st = AfkState::default();
        let work = run(Kind::Activity, "10:00");
        assert!(check(&events, MIN15, Some(&work), t("10:46"), &mut st).is_some());
    }

    #[test]
    fn range_in_local_time() {
        let tz = FixedOffset::east_opt(2 * 3600).unwrap();
        let start: DateTime<Utc> = "2026-10-09T08:03:27Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-10-09T08:45:50Z".parse().unwrap();
        let r = range_text(start, end, &tz);
        assert_eq!(r, "2026-10-09 10:03-10:45");
        let now = DateTime::parse_from_rfc3339("2026-10-09T10:46:00+02:00").unwrap();
        let (s, e) = crate::timeparse::parse_range(&r, &now).unwrap();
        assert_eq!(
            (s, e),
            (
                "2026-10-09T08:03:00Z".parse().unwrap(),
                "2026-10-09T08:45:00Z".parse().unwrap()
            )
        );
    }
}
