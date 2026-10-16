//! Parser for time ranges of retroactive entries.
//!
//! Supported expressions (case-insensitive):
//!
//! | Expression                      | Meaning                                   |
//! |---------------------------------|-------------------------------------------|
//! | `-30m`, `30m`, `last 1h30m`     | the last 30 minutes / 1.5 hours until now |
//! | `10:00-11:15`, `10:00 to 11:15` | absolute range                            |
//! | `since 9:30`                    | from 9:30 until now                       |
//! | `10:00 +45m`, `10:00 for 45m`   | start plus duration                       |
//! | `yesterday 16:00-17:30`         | a range on another day                    |
//! | `2026-10-09 10:00-11:00`        | a range on a specific date                |
//!
//! Clock times may use 24h (`14:30`) or 12h (`2:30pm`) notation, optionally
//! with seconds (`14:30:15`); a bare hour (`10`) means `10:00`. A clock time without a date refers to its most recent
//! occurrence: if the start lies in the future today, the range is moved to
//! yesterday (e.g. `23:00-01:00` early in the morning). Ranges must not end
//! in the future and must not be empty.

use std::time::Duration;

use chrono::{DateTime, Days, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

use crate::duration::parse_duration;

/// A point in time as written by the user, before resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum PointSpec {
    Now,
    /// `-30m`: the given duration before now.
    Ago(Duration),
    /// A full timestamp with time zone (RFC 3339).
    Absolute(DateTime<Utc>),
    /// A clock time, optionally with a date.
    Clock {
        date: Option<DateSpec>,
        time: NaiveTime,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DateSpec {
    Today,
    Yesterday,
    Date(NaiveDate),
}

/// The end of a range as written by the user.
#[derive(Debug, Clone, PartialEq)]
pub enum EndSpec {
    Point(PointSpec),
    Duration(Duration),
}

const HELP: &str = "expected e.g. -30m, 10:00-11:15, since 9:30 or 10:00 +45m";

fn parse_clock(s: &str) -> Option<NaiveTime> {
    let s = s.trim();
    let (body, meridiem) = if let Some(b) = s.strip_suffix("am") {
        (b.trim_end(), Some(false))
    } else if let Some(b) = s.strip_suffix("pm") {
        (b.trim_end(), Some(true))
    } else {
        (s, None)
    };
    if body.is_empty()
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
    {
        return None;
    }
    let parts: Vec<&str> = body.split([':', '.']).collect();
    let (h, m, sec) = match parts.as_slice() {
        [h] if !h.is_empty() && h.len() <= 2 => (h.parse::<u32>().ok()?, 0, 0),
        [h, m] if !h.is_empty() && h.len() <= 2 && m.len() == 2 => {
            (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?, 0)
        }
        [h, m, sec] if !h.is_empty() && h.len() <= 2 && m.len() == 2 && sec.len() == 2 => (
            h.parse::<u32>().ok()?,
            m.parse::<u32>().ok()?,
            sec.parse::<u32>().ok()?,
        ),
        _ => return None,
    };
    if m > 59 || sec > 59 {
        return None;
    }
    let h = match meridiem {
        None => {
            if h > 23 {
                return None;
            }
            h
        }
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            match (h, pm) {
                (12, false) => 0,
                (12, true) => 12,
                (h, false) => h,
                (h, true) => h + 12,
            }
        }
    };
    NaiveTime::from_hms_opt(h, m, sec)
}

/// Parses a single point in time (`now`, `-30m`, `9:30`, `yesterday 9:30`,
/// `2026-10-09 09:30`, `2026-10-09T09:30`, RFC 3339).
pub fn parse_point(input: &str) -> Result<PointSpec, String> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return Err("empty time".into());
    }
    if s == "now" {
        return Ok(PointSpec::Now);
    }
    if let Some(rest) = s.strip_prefix('-') {
        return parse_duration(rest)
            .map(PointSpec::Ago)
            .map_err(|_| format!("invalid time \"{}\"", input.trim()));
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(input.trim()) {
        return Ok(PointSpec::Absolute(dt.with_timezone(&Utc)));
    }
    for fmt in ["%Y-%m-%dt%H:%M", "%Y-%m-%dt%H:%M:%S"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(&s, fmt) {
            return Ok(PointSpec::Clock {
                date: Some(DateSpec::Date(ndt.date())),
                time: ndt.time(),
            });
        }
    }
    let (date, clock) = if let Some(rest) = s.strip_prefix("yesterday") {
        (Some(DateSpec::Yesterday), rest)
    } else if let Some(rest) = s.strip_prefix("today") {
        (Some(DateSpec::Today), rest)
    } else if s.len() >= 10
        && s.is_char_boundary(10)
        && NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").is_ok()
    {
        let d = NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").unwrap();
        (Some(DateSpec::Date(d)), &s[10..])
    } else {
        (None, s.as_str())
    };
    if date.is_some() && !clock.is_empty() && !clock.starts_with(' ') {
        return Err(format!("invalid time \"{}\"", input.trim()));
    }
    let time = parse_clock(clock).ok_or_else(|| format!("invalid time \"{}\"", input.trim()))?;
    Ok(PointSpec::Clock { date, time })
}

/// Splits a range expression into start and end specs.
pub fn parse_range_spec(input: &str) -> Result<(PointSpec, EndSpec), String> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return Err(format!("empty time range ({HELP})"));
    }
    if let Some(rest) = s.strip_prefix("last ") {
        let d = parse_duration(rest).map_err(|e| format!("{e} ({HELP})"))?;
        return Ok((PointSpec::Ago(d), EndSpec::Point(PointSpec::Now)));
    }
    if let Some(rest) = s.strip_prefix("since ") {
        return Ok((parse_point(rest)?, EndSpec::Point(PointSpec::Now)));
    }
    // A plain duration ("-30m" or "30m") means "the last ...".
    if let Ok(d) = parse_duration(s.strip_prefix('-').unwrap_or(&s)) {
        return Ok((PointSpec::Ago(d), EndSpec::Point(PointSpec::Now)));
    }
    if let Some((a, b)) = s.split_once(" for ") {
        let d = parse_duration(b).map_err(|e| format!("{e} ({HELP})"))?;
        return Ok((parse_point(a)?, EndSpec::Duration(d)));
    }
    if let Some((a, b)) = s.rsplit_once('+') {
        if let Ok(d) = parse_duration(b) {
            return Ok((parse_point(a)?, EndSpec::Duration(d)));
        }
    }
    if let Some((a, b)) = s.split_once(" to ") {
        return Ok((parse_point(a)?, EndSpec::Point(parse_end_point(b)?)));
    }
    for sep in ['–', '—'] {
        if let Some((a, b)) = s.split_once(sep) {
            return Ok((parse_point(a)?, EndSpec::Point(parse_end_point(b)?)));
        }
    }
    // Try every '-' as separator; dates and relative times also contain '-'.
    for (i, _) in s.match_indices('-') {
        let (a, b) = (&s[..i], &s[i + 1..]);
        if a.trim().is_empty() {
            continue;
        }
        if let (Ok(start), Ok(end)) = (parse_point(a), parse_end_point(b)) {
            return Ok((start, EndSpec::Point(end)));
        }
    }
    Err(format!("invalid time range \"{}\" ({HELP})", input.trim()))
}

fn parse_end_point(s: &str) -> Result<PointSpec, String> {
    parse_point(s)
}

fn local_at<Tz: TimeZone>(
    tz: &Tz,
    date: NaiveDate,
    time: NaiveTime,
) -> Result<DateTime<Tz>, String> {
    tz.from_local_datetime(&date.and_time(time))
        .earliest()
        .ok_or_else(|| {
            format!(
                "{} {} does not exist in the local time zone",
                date,
                time.format("%H:%M")
            )
        })
}

fn resolve_date<Tz: TimeZone>(spec: DateSpec, now: &DateTime<Tz>) -> NaiveDate {
    let today = now.date_naive();
    match spec {
        DateSpec::Today => today,
        DateSpec::Yesterday => today - Days::new(1),
        DateSpec::Date(d) => d,
    }
}

/// Resolves start and end specs into an absolute, validated range.
pub fn resolve_range<Tz: TimeZone>(
    start: &PointSpec,
    end: &EndSpec,
    now: &DateTime<Tz>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let tz = now.timezone();
    let now_utc = now.with_timezone(&Utc);
    let (start_dt, start_date): (DateTime<Utc>, NaiveDate) = match start {
        PointSpec::Now => (now_utc, now.date_naive()),
        PointSpec::Ago(d) => {
            let t = now_utc - chrono::Duration::from_std(*d).map_err(|e| e.to_string())?;
            (t, t.with_timezone(&tz).date_naive())
        }
        PointSpec::Absolute(t) => (*t, t.with_timezone(&tz).date_naive()),
        PointSpec::Clock {
            date: Some(d),
            time,
        } => {
            let date = resolve_date(*d, now);
            (local_at(&tz, date, *time)?.with_timezone(&Utc), date)
        }
        PointSpec::Clock { date: None, time } => {
            let mut date = now.date_naive();
            let mut t = local_at(&tz, date, *time)?.with_timezone(&Utc);
            if t > now_utc {
                date = date - Days::new(1);
                t = local_at(&tz, date, *time)?.with_timezone(&Utc);
            }
            (t, date)
        }
    };
    let end_dt: DateTime<Utc> = match end {
        EndSpec::Duration(d) => {
            start_dt + chrono::Duration::from_std(*d).map_err(|e| e.to_string())?
        }
        EndSpec::Point(PointSpec::Now) => now_utc,
        EndSpec::Point(PointSpec::Ago(d)) => {
            now_utc - chrono::Duration::from_std(*d).map_err(|e| e.to_string())?
        }
        EndSpec::Point(PointSpec::Absolute(t)) => *t,
        EndSpec::Point(PointSpec::Clock {
            date: Some(d),
            time,
        }) => local_at(&tz, resolve_date(*d, now), *time)?.with_timezone(&Utc),
        EndSpec::Point(PointSpec::Clock { date: None, time }) => {
            let mut t = local_at(&tz, start_date, *time)?.with_timezone(&Utc);
            if t <= start_dt {
                t = local_at(&tz, start_date + Days::new(1), *time)?.with_timezone(&Utc);
            }
            t
        }
    };
    if end_dt <= start_dt {
        return Err("the end of the range must be after its start".into());
    }
    if end_dt > now_utc {
        let crosses_midnight = matches!(end, EndSpec::Point(PointSpec::Clock { date: None, .. }))
            && end_dt.with_timezone(&tz).date_naive() != start_date;
        return Err(if crosses_midnight {
            "the range must not end in the future (an end before the start means the next day)"
                .into()
        } else {
            "the range must not end in the future".into()
        });
    }
    Ok((start_dt, end_dt))
}

/// Parses and resolves a range expression relative to `now`.
pub fn parse_range<Tz: TimeZone>(
    input: &str,
    now: &DateTime<Tz>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let (start, end) = parse_range_spec(input)?;
    resolve_range(&start, &end, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn now() -> DateTime<FixedOffset> {
        // 2026-10-09 14:00 local (+02:00)
        DateTime::parse_from_rfc3339("2026-10-09T14:00:00+02:00").unwrap()
    }

    fn r(expr: &str) -> (String, String) {
        let (a, b) = parse_range(expr, &now()).unwrap_or_else(|e| panic!("{expr}: {e}"));
        let off = now().timezone();
        (
            a.with_timezone(&off).format("%m-%d %H:%M").to_string(),
            b.with_timezone(&off).format("%m-%d %H:%M").to_string(),
        )
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn relative() {
        assert_eq!(r("-30m"), pair("10-09 13:30", "10-09 14:00"));
        assert_eq!(r("30m"), pair("10-09 13:30", "10-09 14:00"));
        assert_eq!(r("last 1h30m"), pair("10-09 12:30", "10-09 14:00"));
        assert_eq!(r("-1.5h"), pair("10-09 12:30", "10-09 14:00"));
    }

    #[test]
    fn absolute_ranges() {
        assert_eq!(r("10:00-11:15"), pair("10-09 10:00", "10-09 11:15"));
        assert_eq!(r("10:00 - 11:15"), pair("10-09 10:00", "10-09 11:15"));
        assert_eq!(r("10:00 to 11:15"), pair("10-09 10:00", "10-09 11:15"));
        assert_eq!(r("10:00–11:15"), pair("10-09 10:00", "10-09 11:15"));
        assert_eq!(r("9-10"), pair("10-09 09:00", "10-09 10:00"));
        assert_eq!(r("9am-10:30am"), pair("10-09 09:00", "10-09 10:30"));
        assert_eq!(r("12pm-1pm"), pair("10-09 12:00", "10-09 13:00"));
        assert_eq!(r("9.30-10.00"), pair("10-09 09:30", "10-09 10:00"));
        let (a, b) = parse_range("2026-10-09 10:03:27-10:45:05", &now()).unwrap();
        assert_eq!(
            (a, b),
            (
                "2026-10-09T08:03:27Z".parse().unwrap(),
                "2026-10-09T08:45:05Z".parse().unwrap()
            )
        );
        assert!(parse_range("10:03:60-11:00", &now()).is_err());
        assert!(parse_range("10:03:5-11:00", &now()).is_err());
    }

    #[test]
    fn since_and_duration() {
        assert_eq!(r("since 9:30"), pair("10-09 09:30", "10-09 14:00"));
        assert_eq!(r("SINCE 9:30"), pair("10-09 09:30", "10-09 14:00"));
        assert_eq!(r("10:00 +45m"), pair("10-09 10:00", "10-09 10:45"));
        assert_eq!(r("10:00+45m"), pair("10-09 10:00", "10-09 10:45"));
        assert_eq!(r("10:00 for 1h"), pair("10-09 10:00", "10-09 11:00"));
    }

    #[test]
    fn dates() {
        assert_eq!(
            r("yesterday 16:00-17:30"),
            pair("10-08 16:00", "10-08 17:30")
        );
        assert_eq!(
            r("2026-10-07 10:00-11:00"),
            pair("10-07 10:00", "10-07 11:00")
        );
        assert_eq!(
            r("2026-10-07 23:00-01:00"),
            pair("10-07 23:00", "10-08 01:00")
        );
        assert_eq!(
            r("2026-10-07T10:00 - 2026-10-07T12:00"),
            pair("10-07 10:00", "10-07 12:00")
        );
        assert_eq!(
            r("since yesterday 18:00"),
            pair("10-08 18:00", "10-09 14:00")
        );
        assert_eq!(
            r("2026-10-09T08:00:00Z to 2026-10-09T09:00:00Z"),
            pair("10-09 10:00", "10-09 11:00")
        );
    }

    #[test]
    fn future_start_moves_to_yesterday() {
        assert_eq!(r("23:00-01:00"), pair("10-08 23:00", "10-09 01:00"));
        assert_eq!(r("since 15:00"), pair("10-08 15:00", "10-09 14:00"));
        assert_eq!(r("16:00-17:00"), pair("10-08 16:00", "10-08 17:00"));
    }

    #[test]
    fn errors() {
        for bad in [
            "",
            "abc",
            "13:00-15:00",
            "10:00-10:00",
            "25:00-26:00",
            "10:61-11:00",
            "since",
            "10:00 +",
            "13pm-2pm",
            "today 15:00-16:00",
        ] {
            assert!(parse_range(bad, &now()).is_err(), "{bad} should fail");
        }
        assert!(parse_range("13:00-15:00", &now())
            .unwrap_err()
            .contains("future"));
    }

    #[test]
    fn points() {
        assert_eq!(parse_point("now").unwrap(), PointSpec::Now);
        assert_eq!(
            parse_point("-30m").unwrap(),
            PointSpec::Ago(Duration::from_secs(1800))
        );
        assert!(matches!(
            parse_point("2026-10-09T10:00:00Z").unwrap(),
            PointSpec::Absolute(_)
        ));
        assert!(matches!(
            parse_point("yesterday 9:00").unwrap(),
            PointSpec::Clock {
                date: Some(DateSpec::Yesterday),
                ..
            }
        ));
        assert!(parse_point("yesterday9:00").is_err());
        assert!(parse_point("123").is_err());
    }

    #[test]
    fn explicit_points() {
        let n = now();
        let (a, b) = resolve_range(
            &parse_point("-1h").unwrap(),
            &EndSpec::Point(parse_point("-30m").unwrap()),
            &n,
        )
        .unwrap();
        assert_eq!((b - a).num_minutes(), 30);
        let (a, b) = resolve_range(
            &parse_point("10:00").unwrap(),
            &EndSpec::Duration(Duration::from_secs(600)),
            &n,
        )
        .unwrap();
        assert_eq!((b - a).num_minutes(), 10);
    }
}
