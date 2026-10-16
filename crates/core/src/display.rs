//! Text shown for the running activity (tray label, tooltip, `status`).

use chrono::{DateTime, Local, Utc};

use crate::config::{Config, TrayDisplay};
use crate::engine::RunningInfo;
use crate::format::{format_clock, format_elapsed, Template, TemplateVars, PATH_SEPARATOR};

pub fn vars(cfg: &Config, r: &RunningInfo, now: DateTime<Utc>) -> TemplateVars {
    TemplateVars::new(
        &r.title,
        &r.path,
        r.note.as_deref(),
        r.start,
        now,
        cfg.format.elapsed,
        cfg.format.time,
    )
}

/// Text next to the tray icon, `None` for `display: icon`.
pub fn tray_text(cfg: &Config, r: &RunningInfo, now: DateTime<Utc>) -> Option<String> {
    let v = vars(cfg, r, now);
    match cfg.tray.display {
        TrayDisplay::Icon => None,
        TrayDisplay::Elapsed => Some(v.elapsed),
        TrayDisplay::Since => Some(format!("since {}", v.since)),
        TrayDisplay::Title => Some(v.title),
        TrayDisplay::Template => Some(
            Template::parse(&cfg.tray.template)
                .map(|t| t.render(&v))
                .unwrap_or_default(),
        ),
    }
}

/// Whether the text changes every second (otherwise once per minute, except
/// for the first minute of a run, which `1h 23m` shows in seconds).
pub fn needs_seconds(cfg: &Config) -> bool {
    if !cfg.format.elapsed.has_seconds() {
        return false;
    }
    match cfg.tray.display {
        TrayDisplay::Elapsed => true,
        TrayDisplay::Template => Template::parse(&cfg.tray.template)
            .map(|t| t.uses("elapsed"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Full information, independent of the display mode (tooltip).
pub fn tooltip(cfg: &Config, r: &RunningInfo, now: DateTime<Utc>) -> String {
    let mut s = format!(
        "{}\nsince {} ({})",
        r.path.join(PATH_SEPARATOR),
        format_clock(&r.start.with_timezone(&Local), cfg.format.time),
        format_elapsed((now - r.start).num_seconds(), cfg.format.elapsed)
    );
    if let Some(n) = &r.note {
        s.push_str(&format!("\nNote: {n}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::favorites::Kind;
    use crate::format::ElapsedFormat;
    use crate::model::{Origin, Source};

    fn running() -> RunningInfo {
        RunningInfo {
            id: "acme/dev".into(),
            title: "Development".into(),
            path: vec!["ACME".into(), "Development".into()],
            kind: Kind::Activity,
            origin: Origin::Config,
            note: Some("nav".into()),
            source: Source::Cli,
            start: "2026-10-09T08:00:00Z".parse().unwrap(),
            elapsed: 0,
            color: None,
            icon: None,
        }
    }

    #[test]
    fn modes() {
        let now: DateTime<Utc> = "2026-10-09T09:23:10Z".parse().unwrap();
        let mut cfg = Config::default();
        assert_eq!(
            tray_text(&cfg, &running(), now).unwrap(),
            "Development · 1h 23m"
        );
        cfg.tray.display = TrayDisplay::Elapsed;
        assert_eq!(tray_text(&cfg, &running(), now).unwrap(), "1h 23m");
        cfg.tray.display = TrayDisplay::Title;
        assert_eq!(tray_text(&cfg, &running(), now).unwrap(), "Development");
        cfg.tray.display = TrayDisplay::Icon;
        assert_eq!(tray_text(&cfg, &running(), now), None);
        cfg.tray.display = TrayDisplay::Since;
        assert!(tray_text(&cfg, &running(), now)
            .unwrap()
            .starts_with("since "));
        cfg.tray.display = TrayDisplay::Template;
        cfg.tray.template = "{path} | {note}".into();
        assert_eq!(
            tray_text(&cfg, &running(), now).unwrap(),
            "ACME › Development | nav"
        );
        let tip = tooltip(&cfg, &running(), now);
        assert!(
            tip.contains("ACME › Development")
                && tip.contains("Note: nav")
                && tip.contains("1h 23m")
        );
    }

    #[test]
    fn seconds() {
        let mut cfg = Config::default();
        assert!(!needs_seconds(&cfg));
        cfg.format.elapsed = ElapsedFormat::ClockSeconds;
        assert!(needs_seconds(&cfg));
        cfg.tray.template = "{title}".into();
        assert!(!needs_seconds(&cfg));
        cfg.tray.display = TrayDisplay::Elapsed;
        assert!(needs_seconds(&cfg));
    }
}
