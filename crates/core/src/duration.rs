//! Human-friendly durations such as `90s`, `15m`, `1h30m` or `1.5h`.

use std::time::Duration;

/// Parses a duration made of one or more `<number><unit>` parts.
///
/// Units: `d`, `h`, `m`, `s`, `ms`. Whitespace between parts is allowed
/// (`1h 30m`). A bare number without a unit is rejected to avoid ambiguity.
pub fn parse_duration(input: &str) -> Result<Duration, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty duration".into());
    }
    let mut total_ms: f64 = 0.0;
    let mut rest = s;
    let mut parts = 0;
    while !rest.is_empty() {
        rest = rest.trim_start();
        let num_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if num_len == 0 {
            return Err(format!(
                "invalid duration \"{input}\" (expected e.g. 30s, 15m, 1h30m)"
            ));
        }
        let number: f64 = rest[..num_len]
            .parse()
            .map_err(|_| format!("invalid number in duration \"{input}\""))?;
        rest = &rest[num_len..];
        let unit_len = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let unit = &rest[..unit_len];
        let factor = match unit.to_ascii_lowercase().as_str() {
            "ms" => 1.0,
            "s" | "sec" | "secs" => 1_000.0,
            "m" | "min" | "mins" => 60_000.0,
            "h" | "hr" | "hrs" => 3_600_000.0,
            "d" => 86_400_000.0,
            "" => return Err(format!("duration \"{input}\" is missing a unit (s, m, h)")),
            other => {
                return Err(format!(
                    "unknown unit \"{other}\" in duration \"{input}\" (use s, m or h)"
                ))
            }
        };
        total_ms += number * factor;
        rest = &rest[unit_len..];
        parts += 1;
    }
    if parts == 0 {
        return Err(format!("invalid duration \"{input}\""));
    }
    if !total_ms.is_finite() || total_ms > 1e13 {
        return Err(format!("duration \"{input}\" is too large"));
    }
    Ok(Duration::from_millis(total_ms.round() as u64))
}

/// Formats a duration compactly, e.g. `1h30m`, `15m`, `2s`, `1500ms`.
/// The output parses back to the same duration.
pub fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms == 0 {
        return "0s".into();
    }
    if ms % 1000 != 0 {
        return format!("{ms}ms");
    }
    let mut secs = ms / 1000;
    let mut out = String::new();
    let h = secs / 3600;
    secs %= 3600;
    let m = secs / 60;
    let s = secs % 60;
    if h > 0 {
        out.push_str(&format!("{h}h"));
    }
    if m > 0 {
        out.push_str(&format!("{m}m"));
    }
    if s > 0 {
        out.push_str(&format!("{s}s"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        assert_eq!(parse_duration("60s").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("15m").unwrap(), Duration::from_secs(900));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_duration("1h 30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_duration("1.5h").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_duration("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("3H").unwrap(), Duration::from_secs(3 * 3600));
    }

    #[test]
    fn rejects_bad_input() {
        for bad in ["", "abc", "10", "10x", "h", "1h-5m", "-5m"] {
            assert!(parse_duration(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn format_roundtrip() {
        for s in ["1h30m", "15m", "2s", "1500ms", "3h", "1h1m1s"] {
            let d = parse_duration(s).unwrap();
            assert_eq!(format_duration(d), s);
            assert_eq!(parse_duration(&format_duration(d)).unwrap(), d);
        }
        assert_eq!(format_duration(Duration::ZERO), "0s");
    }
}
