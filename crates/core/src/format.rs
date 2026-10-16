//! Formatting of the running activity for the tray and `aw-manual status`.

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Placeholders that can be used in templates.
pub const PLACEHOLDERS: &[&str] = &["title", "path", "elapsed", "since", "note"];

/// Separator used when a path is shown as text.
pub const PATH_SEPARATOR: &str = " › ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ElapsedFormat {
    /// `1h 23m`
    #[serde(rename = "1h 23m")]
    HoursMinutes,
    /// `1:23`
    #[serde(rename = "1:23")]
    Clock,
    /// `1h 23m 45s`
    #[serde(rename = "1h 23m 45s")]
    HoursMinutesSeconds,
    /// `1:23:45`
    #[serde(rename = "1:23:45")]
    ClockSeconds,
}

impl ElapsedFormat {
    pub const ALL: [(&'static str, ElapsedFormat); 4] = [
        ("1h 23m", ElapsedFormat::HoursMinutes),
        ("1:23", ElapsedFormat::Clock),
        ("1h 23m 45s", ElapsedFormat::HoursMinutesSeconds),
        ("1:23:45", ElapsedFormat::ClockSeconds),
    ];

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().find(|(n, _)| *n == s).map(|(_, f)| *f)
    }

    pub fn as_str(&self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, f)| f == self)
            .map(|(n, _)| *n)
            .unwrap()
    }

    pub fn has_seconds(&self) -> bool {
        matches!(
            self,
            ElapsedFormat::HoursMinutesSeconds | ElapsedFormat::ClockSeconds
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeFormat {
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "12h")]
    H12,
}

pub fn format_elapsed(secs: i64, fmt: ElapsedFormat) -> String {
    let secs = secs.max(0);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    match fmt {
        ElapsedFormat::HoursMinutes => {
            if h > 0 {
                format!("{h}h {m}m")
            } else if m > 0 {
                format!("{m}m")
            } else {
                // Below one minute, "0m" says nothing: show the seconds.
                format!("{s}s")
            }
        }
        ElapsedFormat::Clock => format!("{h}:{m:02}"),
        ElapsedFormat::HoursMinutesSeconds => {
            if h > 0 {
                format!("{h}h {m}m {s}s")
            } else if m > 0 {
                format!("{m}m {s}s")
            } else {
                format!("{s}s")
            }
        }
        ElapsedFormat::ClockSeconds => format!("{h}:{m:02}:{s:02}"),
    }
}

pub fn format_clock<Tz: TimeZone>(t: &DateTime<Tz>, fmt: TimeFormat) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match fmt {
        TimeFormat::H24 => t.format("%H:%M").to_string(),
        TimeFormat::H12 => t.format("%-I:%M %p").to_string(),
    }
}

/// A parsed template: literal text and placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Var(String),
}

impl Template {
    /// Parses a template such as `{title} · {elapsed}`. `{{` and `}}` produce
    /// literal braces. Unknown placeholders are an error.
    pub fn parse(src: &str) -> Result<Template, String> {
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut chars = src.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    text.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    text.push('}');
                }
                '{' => {
                    let mut name = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(ch) => name.push(ch),
                            None => {
                                return Err(format!(
                                    "unclosed placeholder \"{{{name}\" in template"
                                ))
                            }
                        }
                    }
                    let name = name.trim().to_string();
                    if !PLACEHOLDERS.contains(&name.as_str()) {
                        return Err(format!(
                            "unknown placeholder {{{name}}} in template (available: {})",
                            PLACEHOLDERS
                                .iter()
                                .map(|p| format!("{{{p}}}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    parts.push(Part::Var(name));
                }
                '}' => {
                    return Err(
                        "unmatched \"}\" in template (use \"}}\" for a literal brace)".into(),
                    )
                }
                c => text.push(c),
            }
        }
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        Ok(Template { parts })
    }

    pub fn uses(&self, var: &str) -> bool {
        self.parts
            .iter()
            .any(|p| matches!(p, Part::Var(v) if v == var))
    }

    pub fn render(&self, vars: &TemplateVars) -> String {
        let mut out = String::new();
        for p in &self.parts {
            match p {
                Part::Text(t) => out.push_str(t),
                Part::Var(v) => out.push_str(match v.as_str() {
                    "title" => &vars.title,
                    "path" => &vars.path,
                    "elapsed" => &vars.elapsed,
                    "since" => &vars.since,
                    "note" => &vars.note,
                    _ => "",
                }),
            }
        }
        out
    }
}

/// Values for template placeholders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateVars {
    pub title: String,
    pub path: String,
    pub elapsed: String,
    pub since: String,
    pub note: String,
}

impl TemplateVars {
    pub fn new(
        title: &str,
        path: &[String],
        note: Option<&str>,
        start: DateTime<Utc>,
        now: DateTime<Utc>,
        elapsed_fmt: ElapsedFormat,
        time_fmt: TimeFormat,
    ) -> Self {
        TemplateVars {
            title: title.to_string(),
            path: path.join(PATH_SEPARATOR),
            elapsed: format_elapsed((now - start).num_seconds(), elapsed_fmt),
            since: format_clock(&start.with_timezone(&Local), time_fmt),
            note: note.unwrap_or_default().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn elapsed() {
        assert_eq!(format_elapsed(0, ElapsedFormat::HoursMinutes), "0s");
        assert_eq!(format_elapsed(42, ElapsedFormat::HoursMinutes), "42s");
        assert_eq!(format_elapsed(60, ElapsedFormat::HoursMinutes), "1m");
        assert_eq!(format_elapsed(23 * 60, ElapsedFormat::HoursMinutes), "23m");
        assert_eq!(
            format_elapsed(3600 + 23 * 60, ElapsedFormat::HoursMinutes),
            "1h 23m"
        );
        assert_eq!(format_elapsed(3600 + 23 * 60, ElapsedFormat::Clock), "1:23");
        assert_eq!(format_elapsed(5 * 60, ElapsedFormat::Clock), "0:05");
        assert_eq!(
            format_elapsed(3600 + 23 * 60 + 45, ElapsedFormat::HoursMinutesSeconds),
            "1h 23m 45s"
        );
        assert_eq!(
            format_elapsed(45, ElapsedFormat::HoursMinutesSeconds),
            "45s"
        );
        assert_eq!(
            format_elapsed(3600 + 23 * 60 + 5, ElapsedFormat::ClockSeconds),
            "1:23:05"
        );
        assert_eq!(format_elapsed(-5, ElapsedFormat::Clock), "0:00");
        assert_eq!(
            format_elapsed(26 * 3600, ElapsedFormat::HoursMinutes),
            "26h 0m"
        );
    }

    #[test]
    fn clock() {
        let t = DateTime::parse_from_rfc3339("2026-10-09T09:30:00+02:00").unwrap();
        assert_eq!(format_clock(&t, TimeFormat::H24), "09:30");
        assert_eq!(format_clock(&t, TimeFormat::H12), "9:30 AM");
        let t =
            t.with_timezone(&FixedOffset::east_opt(2 * 3600).unwrap()) + chrono::Duration::hours(5);
        assert_eq!(format_clock(&t, TimeFormat::H12), "2:30 PM");
    }

    #[test]
    fn template() {
        let t = Template::parse("{title} · {elapsed}").unwrap();
        let vars = TemplateVars {
            title: "Development".into(),
            elapsed: "1h 23m".into(),
            ..Default::default()
        };
        assert_eq!(t.render(&vars), "Development · 1h 23m");
        assert!(t.uses("elapsed"));
        assert!(!t.uses("note"));
        assert_eq!(
            Template::parse("{{x}} {note}")
                .unwrap()
                .render(&TemplateVars {
                    note: "n".into(),
                    ..Default::default()
                }),
            "{x} n"
        );
        assert!(Template::parse("{foo}").is_err());
        assert!(Template::parse("{title").is_err());
        assert!(Template::parse("title}").is_err());
        assert_eq!(Template::parse("").unwrap().render(&vars), "");
    }

    #[test]
    fn elapsed_format_names() {
        for (name, f) in ElapsedFormat::ALL {
            assert_eq!(ElapsedFormat::parse(name), Some(f));
            assert_eq!(f.as_str(), name);
        }
        assert_eq!(ElapsedFormat::parse("xx"), None);
    }
}
