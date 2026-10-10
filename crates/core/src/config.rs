//! Base configuration (`config.yaml`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{NaiveTime, Weekday};
use serde::Serialize;

use crate::duration::{format_duration, parse_duration};
use crate::format::{ElapsedFormat, Template, TimeFormat};
use crate::yaml::{self, Node, NodeKind, YamlError};

pub const CONFIG_FILE: &str = "config.yaml";
pub const DEFAULT_FAVORITES_DIR: &str = "favorites.d";
/// Highest `schema_version` this build understands.
pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Config {
    pub server: ServerConfig,
    pub favorites: FavoritesConfig,
    pub hotkeys: HotkeysConfig,
    pub tray: TrayConfig,
    pub notifications: NotificationsConfig,
    pub popover: PopoverConfig,
    pub cli: CliConfig,
    pub log: LogConfig,
    pub openaction: OpenActionConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServerConfig {
    pub url: String,
    /// Bucket ID; `{hostname}` is replaced by the local host name.
    pub bucket_id: String,
    pub bucket_type: String,
    #[serde(serialize_with = "ser_duration")]
    pub heartbeat_interval: Duration,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FavoritesConfig {
    pub dirs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HotkeysConfig {
    pub popover: Option<String>,
    pub stop: Option<String>,
    pub resume: Option<String>,
    /// Hotkeys for individual favorites: (favorite id, hotkey), in written order.
    pub favorites: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrayDisplay {
    Icon,
    Elapsed,
    Since,
    Title,
    Template,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayConfig {
    pub display: TrayDisplay,
    pub template: String,
    pub elapsed_format: ElapsedFormat,
    pub time_format: TimeFormat,
    pub color_icon: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkHours {
    #[serde(serialize_with = "ser_days")]
    pub days: Vec<Weekday>,
    #[serde(serialize_with = "ser_time")]
    pub from: NaiveTime,
    #[serde(serialize_with = "ser_time")]
    pub to: NaiveTime,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NotificationsConfig {
    /// `None` = disabled.
    #[serde(serialize_with = "ser_opt_duration")]
    pub idle_reminder: Option<Duration>,
    #[serde(serialize_with = "ser_opt_duration")]
    pub long_running: Option<Duration>,
    pub after_restart: bool,
    pub offline: bool,
    pub work_hours: WorkHours,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StartAt {
    Running,
    First,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScrollConfig {
    pub confirm_with_hotkey: bool,
    pub dwell_confirm: bool,
    #[serde(serialize_with = "ser_duration")]
    pub dwell_time: Duration,
    #[serde(serialize_with = "ser_duration")]
    pub timeout: Duration,
    pub sensitivity: f64,
    pub wrap_around: bool,
    pub open_at_cursor: bool,
    pub start_at: StartAt,
    /// Horizontal scrolling / wheel tilt acts as back (left) and confirm (right).
    pub horizontal: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PopoverConfig {
    pub scroll: ScrollConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CliConfig {
    /// Start the daemon automatically when a CLI command needs it.
    pub autostart_daemon: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogConfig {
    pub level: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToggleRunning {
    Stop,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenActionConfig {
    pub toggle_running: ToggleRunning,
}

fn ser_duration<S: serde::Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format_duration(*d))
}

fn ser_opt_duration<S: serde::Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
    match d {
        Some(d) => s.serialize_str(&format_duration(*d)),
        None => s.serialize_bool(false),
    }
}

fn ser_time<S: serde::Serializer>(t: &NaiveTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&t.format("%H:%M").to_string())
}

fn ser_days<S: serde::Serializer>(d: &[Weekday], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(d.len()))?;
    for day in d {
        seq.serialize_element(weekday_name(*day))?;
    }
    seq.end()
}

pub fn weekday_name(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    Some(match s.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server: ServerConfig {
                url: "http://localhost:5600".into(),
                bucket_id: "aw-manual_{hostname}".into(),
                bucket_type: "manual.activity".into(),
                heartbeat_interval: Duration::from_secs(60),
            },
            favorites: FavoritesConfig {
                dirs: vec![DEFAULT_FAVORITES_DIR.into()],
            },
            hotkeys: HotkeysConfig {
                popover: Some("Ctrl+Alt+Space".into()),
                stop: Some("Ctrl+Alt+S".into()),
                resume: Some("Ctrl+Alt+R".into()),
                favorites: Vec::new(),
            },
            tray: TrayConfig {
                display: TrayDisplay::Template,
                template: "{title} · {elapsed}".into(),
                elapsed_format: ElapsedFormat::HoursMinutes,
                time_format: TimeFormat::H24,
                color_icon: true,
            },
            notifications: NotificationsConfig {
                idle_reminder: Some(Duration::from_secs(15 * 60)),
                long_running: Some(Duration::from_secs(3 * 3600)),
                after_restart: true,
                offline: true,
                work_hours: WorkHours {
                    days: vec![
                        Weekday::Mon,
                        Weekday::Tue,
                        Weekday::Wed,
                        Weekday::Thu,
                        Weekday::Fri,
                    ],
                    from: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
                    to: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
                },
            },
            popover: PopoverConfig {
                scroll: ScrollConfig {
                    confirm_with_hotkey: true,
                    dwell_confirm: false,
                    dwell_time: Duration::from_secs(2),
                    timeout: Duration::from_secs(10),
                    sensitivity: 1.0,
                    wrap_around: false,
                    open_at_cursor: true,
                    start_at: StartAt::Running,
                    horizontal: false,
                },
            },
            cli: CliConfig {
                autostart_daemon: true,
            },
            log: LogConfig {
                level: "info".into(),
            },
            openaction: OpenActionConfig {
                toggle_running: ToggleRunning::Stop,
            },
        }
    }
}

/// The commented configuration written on first start.
pub const DEFAULT_CONFIG_YAML: &str = r##"# aw-manual base configuration
# Favorites live in separate files, see favorites.d/
schema_version: 1

server:
  url: http://localhost:5600        # a remote aw-server is possible as well
  bucket_id: "aw-manual_{hostname}"
  heartbeat_interval: 60s

favorites:
  dirs:                             # loaded in this order
    - favorites.d

hotkeys:
  popover: Ctrl+Alt+Space           # alternatively a mouse button, e.g. Mouse4
  stop: Ctrl+Alt+S
  resume: Ctrl+Alt+R

tray:
  display: template                 # icon | elapsed | since | title | template
  template: "{title} · {elapsed}"   # {title} {path} {elapsed} {since} {note}
  elapsed_format: 1h 23m            # "1h 23m" | "1:23" | "1h 23m 45s" | "1:23:45"
  time_format: 24h                  # 24h | 12h
  color_icon: true                  # tint the icon in the favorite's color

notifications:
  idle_reminder: 15m                # false to disable
  long_running: 3h                  # false to disable
  after_restart: true
  offline: true
  work_hours:
    days: [mon, tue, wed, thu, fri]
    from: "08:00"
    to: "18:00"

popover:
  scroll:
    confirm_with_hotkey: true
    dwell_confirm: false            # confirm by dwell time
    dwell_time: 2s
    timeout: 10s                    # must be longer than dwell_time
    sensitivity: 1.0                # threshold factor per step
    wrap_around: false
    open_at_cursor: true
    start_at: running               # running | first
"##;

/// Collects validation errors for one file.
struct Errors(Vec<YamlError>);

impl Errors {
    fn push(&mut self, node: &Node, msg: impl Into<String>) {
        self.0.push(YamlError::at(node, msg));
    }
}

/// Reads the entries of a mapping section, reporting unknown keys.
fn section<'a>(
    node: &'a Node,
    path: &str,
    known: &[&str],
    errs: &mut Errors,
) -> Option<&'a [(Node, Node)]> {
    match &node.kind {
        NodeKind::Null => None,
        NodeKind::Map { entries, .. } => {
            for (k, _) in entries {
                let key = k.as_scalar().unwrap_or_default();
                if !known.contains(&key) {
                    errs.push(
                        k,
                        format!(
                            "unknown setting \"{}{key}\" (expected one of: {})",
                            prefix(path),
                            known.join(", ")
                        ),
                    );
                }
            }
            Some(entries)
        }
        _ => {
            let name = if path.is_empty() {
                "the file".to_string()
            } else {
                format!("\"{path}\"")
            };
            errs.push(
                node,
                format!("{name} must be a mapping, found {}", node.type_name()),
            );
            None
        }
    }
}

fn prefix(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else {
        format!("{path}.")
    }
}

fn entry<'a>(entries: Option<&'a [(Node, Node)]>, key: &str) -> Option<&'a Node> {
    entries?
        .iter()
        .find(|(k, _)| k.as_scalar() == Some(key))
        .map(|(_, v)| v)
}

fn read_string(node: Option<&Node>, path: &str, target: &mut String, errs: &mut Errors) {
    if let Some(n) = node {
        match n.as_scalar() {
            Some(s) => *target = s.to_string(),
            None => errs.push(
                n,
                format!("\"{path}\" must be text, found {}", n.type_name()),
            ),
        }
    }
}

fn read_opt_string(
    node: Option<&Node>,
    path: &str,
    target: &mut Option<String>,
    errs: &mut Errors,
) {
    if let Some(n) = node {
        if n.is_null() || n.as_bool() == Some(false) || n.as_scalar() == Some("") {
            *target = None;
        } else {
            match n.as_scalar() {
                Some(s) => *target = Some(s.trim().to_string()),
                None => errs.push(
                    n,
                    format!("\"{path}\" must be text, found {}", n.type_name()),
                ),
            }
        }
    }
}

fn read_bool(node: Option<&Node>, path: &str, target: &mut bool, errs: &mut Errors) {
    if let Some(n) = node {
        match n.as_bool() {
            Some(b) => *target = b,
            None => errs.push(n, format!("\"{path}\" must be true or false")),
        }
    }
}

fn read_duration(node: Option<&Node>, path: &str, target: &mut Duration, errs: &mut Errors) {
    if let Some(n) = node {
        match n.as_scalar().map(parse_duration) {
            Some(Ok(d)) => *target = d,
            Some(Err(e)) => errs.push(n, format!("\"{path}\": {e}")),
            None => errs.push(
                n,
                format!("\"{path}\" must be a duration such as 30s, 15m or 1h"),
            ),
        }
    }
}

fn read_opt_duration(
    node: Option<&Node>,
    path: &str,
    target: &mut Option<Duration>,
    errs: &mut Errors,
) {
    if let Some(n) = node {
        if n.as_bool() == Some(false) || matches!(n.as_scalar(), Some("off")) {
            *target = None;
            return;
        }
        match n.as_scalar().map(parse_duration) {
            Some(Ok(d)) if !d.is_zero() => *target = Some(d),
            Some(Ok(_)) => errs.push(
                n,
                format!("\"{path}\" must be longer than 0 (use false to disable)"),
            ),
            Some(Err(e)) => errs.push(n, format!("\"{path}\": {e} (use false to disable)")),
            None => errs.push(
                n,
                format!("\"{path}\" must be a duration such as 15m, or false"),
            ),
        }
    }
}

fn read_enum<T: Copy>(
    node: Option<&Node>,
    path: &str,
    options: &[(&str, T)],
    target: &mut T,
    errs: &mut Errors,
) {
    if let Some(n) = node {
        let value = n.as_scalar().unwrap_or_default();
        match options.iter().find(|(name, _)| *name == value) {
            Some((_, v)) => *target = *v,
            None => errs.push(
                n,
                format!(
                    "\"{path}\" must be one of: {}",
                    options
                        .iter()
                        .map(|(n, _)| format!("\"{n}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        }
    }
}

fn read_clock(node: Option<&Node>, path: &str, target: &mut NaiveTime, errs: &mut Errors) {
    if let Some(n) = node {
        match n
            .as_scalar()
            .and_then(|s| NaiveTime::parse_from_str(s.trim(), "%H:%M").ok())
        {
            Some(t) => *target = t,
            None => errs.push(
                n,
                format!("\"{path}\" must be a time like \"08:00\" (quoted)"),
            ),
        }
    }
}

/// Checks the syntax of a hotkey such as `Ctrl+Alt+Space` or `Mouse4`.
pub fn validate_hotkey(s: &str) -> Result<(), String> {
    let s = s.trim();
    if let Some(n) = s.strip_prefix("Mouse").or_else(|| s.strip_prefix("mouse")) {
        return match n.parse::<u8>() {
            Ok(4..=9) => Ok(()),
            _ => Err(format!(
                "invalid mouse button \"{s}\" (use Mouse4 … Mouse9)"
            )),
        };
    }
    const MODIFIERS: &[&str] = &[
        "ctrl",
        "control",
        "alt",
        "option",
        "shift",
        "super",
        "cmd",
        "command",
        "commandorcontrol",
        "commandorctrl",
        "cmdorctrl",
        "cmdorcontrol",
    ];
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("invalid hotkey \"{s}\""));
    }
    let keys: Vec<&&str> = parts
        .iter()
        .filter(|p| !MODIFIERS.contains(&p.to_ascii_lowercase().as_str()))
        .collect();
    match keys.len() {
        1 if !MODIFIERS.contains(&parts[parts.len() - 1].to_ascii_lowercase().as_str()) => Ok(()),
        1 => Err(format!(
            "hotkey \"{s}\": the key must come after the modifiers"
        )),
        0 => Err(format!("hotkey \"{s}\" has no key, only modifiers")),
        _ => Err(format!(
            "hotkey \"{s}\" must contain exactly one non-modifier key"
        )),
    }
}

/// Parses `config.yaml` content. Returns all validation errors at once.
pub fn parse_config(src: &str) -> Result<Config, Vec<YamlError>> {
    let root = yaml::parse(src).map_err(|e| vec![e])?;
    let mut cfg = Config::default();
    let mut errs = Errors(Vec::new());
    let top = section(
        &root,
        "",
        &[
            "schema_version",
            "server",
            "favorites",
            "hotkeys",
            "tray",
            "notifications",
            "popover",
            "cli",
            "log",
            "openaction",
        ],
        &mut errs,
    );
    if !root.is_null() && top.is_none() && errs.0.is_empty() {
        errs.push(&root, "config.yaml must contain a mapping");
    }

    if let Some(n) = entry(top, "schema_version") {
        match n.as_i64() {
            Some(v) if (1..=SCHEMA_VERSION).contains(&v) => {}
            Some(v) => errs.push(
                n,
                format!(
                    "unsupported schema_version {v} (this version supports up to {SCHEMA_VERSION})"
                ),
            ),
            None => errs.push(n, "schema_version must be a number"),
        }
    }

    if let Some(server_node) = entry(top, "server") {
        let s = section(
            server_node,
            "server",
            &["url", "bucket_id", "bucket_type", "heartbeat_interval"],
            &mut errs,
        );
        read_string(
            entry(s, "url"),
            "server.url",
            &mut cfg.server.url,
            &mut errs,
        );
        if let Some(n) = entry(s, "url") {
            if let Err(e) = validate_server_url(&cfg.server.url) {
                errs.push(n, e);
            }
        }
        read_string(
            entry(s, "bucket_id"),
            "server.bucket_id",
            &mut cfg.server.bucket_id,
            &mut errs,
        );
        if let Some(n) = entry(s, "bucket_id") {
            if cfg.server.bucket_id.trim().is_empty() {
                errs.push(n, "server.bucket_id must not be empty");
            } else if cfg.server.bucket_id.contains('/') {
                errs.push(n, "server.bucket_id must not contain \"/\"");
            }
        }
        read_string(
            entry(s, "bucket_type"),
            "server.bucket_type",
            &mut cfg.server.bucket_type,
            &mut errs,
        );
        if let Some(n) = entry(s, "bucket_type") {
            if cfg.server.bucket_type.trim().is_empty() {
                errs.push(n, "server.bucket_type must not be empty");
            }
        }
        read_duration(
            entry(s, "heartbeat_interval"),
            "server.heartbeat_interval",
            &mut cfg.server.heartbeat_interval,
            &mut errs,
        );
        if let Some(n) = entry(s, "heartbeat_interval") {
            let d = cfg.server.heartbeat_interval;
            if d < Duration::from_secs(5) || d > Duration::from_secs(600) {
                errs.push(n, "server.heartbeat_interval must be between 5s and 10m");
            }
        }
    }

    if let Some(fav_node) = entry(top, "favorites") {
        let s = section(fav_node, "favorites", &["dirs"], &mut errs);
        if let Some(n) = entry(s, "dirs") {
            match n.as_seq() {
                Some(items) => {
                    let mut dirs = Vec::new();
                    for item in items {
                        match item.as_scalar() {
                            Some(d) if !d.trim().is_empty() => dirs.push(d.trim().to_string()),
                            _ => errs.push(item, "favorites.dirs entries must be folder paths"),
                        }
                    }
                    cfg.favorites.dirs = dirs;
                }
                None => errs.push(n, "favorites.dirs must be a list of folders"),
            }
        }
    }

    if let Some(hk_node) = entry(top, "hotkeys") {
        let s = section(
            hk_node,
            "hotkeys",
            &["popover", "stop", "resume", "favorites"],
            &mut errs,
        );
        read_opt_string(
            entry(s, "popover"),
            "hotkeys.popover",
            &mut cfg.hotkeys.popover,
            &mut errs,
        );
        read_opt_string(
            entry(s, "stop"),
            "hotkeys.stop",
            &mut cfg.hotkeys.stop,
            &mut errs,
        );
        read_opt_string(
            entry(s, "resume"),
            "hotkeys.resume",
            &mut cfg.hotkeys.resume,
            &mut errs,
        );
        for key in ["popover", "stop", "resume"] {
            let value = match key {
                "popover" => &cfg.hotkeys.popover,
                "stop" => &cfg.hotkeys.stop,
                _ => &cfg.hotkeys.resume,
            };
            if let (Some(v), Some(n)) = (value, entry(s, key)) {
                if let Err(e) = validate_hotkey(v) {
                    errs.push(n, format!("hotkeys.{key}: {e}"));
                }
            }
        }
        if let Some(n) = entry(s, "favorites") {
            match &n.kind {
                NodeKind::Null => {}
                NodeKind::Map { entries, .. } => {
                    for (k, v) in entries {
                        let id = k.as_scalar().unwrap_or_default().to_string();
                        match v.as_scalar() {
                            Some(hk) => match validate_hotkey(hk) {
                                Ok(()) => cfg.hotkeys.favorites.push((id, hk.trim().to_string())),
                                Err(e) => errs.push(v, format!("hotkeys.favorites.{id}: {e}")),
                            },
                            None => {
                                errs.push(v, format!("hotkeys.favorites.{id} must be a hotkey"))
                            }
                        }
                    }
                }
                _ => errs.push(n, "hotkeys.favorites must map favorite IDs to hotkeys"),
            }
        }
        // The same hotkey must not be bound twice.
        let mut seen: Vec<String> = Vec::new();
        let all = [&cfg.hotkeys.popover, &cfg.hotkeys.stop, &cfg.hotkeys.resume]
            .into_iter()
            .flatten()
            .cloned()
            .chain(cfg.hotkeys.favorites.iter().map(|(_, h)| h.clone()));
        for hk in all {
            let norm = hk.to_ascii_lowercase().replace(' ', "");
            if seen.contains(&norm) {
                errs.push(
                    hk_node,
                    format!("hotkey \"{hk}\" is assigned more than once"),
                );
            }
            seen.push(norm);
        }
    }

    if let Some(tray_node) = entry(top, "tray") {
        let s = section(
            tray_node,
            "tray",
            &[
                "display",
                "template",
                "elapsed_format",
                "time_format",
                "color_icon",
            ],
            &mut errs,
        );
        read_enum(
            entry(s, "display"),
            "tray.display",
            &[
                ("icon", TrayDisplay::Icon),
                ("elapsed", TrayDisplay::Elapsed),
                ("since", TrayDisplay::Since),
                ("title", TrayDisplay::Title),
                ("template", TrayDisplay::Template),
            ],
            &mut cfg.tray.display,
            &mut errs,
        );
        read_string(
            entry(s, "template"),
            "tray.template",
            &mut cfg.tray.template,
            &mut errs,
        );
        if let Some(n) = entry(s, "template") {
            if let Err(e) = Template::parse(&cfg.tray.template) {
                errs.push(n, format!("tray.template: {e}"));
            }
        }
        read_enum(
            entry(s, "elapsed_format"),
            "tray.elapsed_format",
            &ElapsedFormat::ALL,
            &mut cfg.tray.elapsed_format,
            &mut errs,
        );
        read_enum(
            entry(s, "time_format"),
            "tray.time_format",
            &[("24h", TimeFormat::H24), ("12h", TimeFormat::H12)],
            &mut cfg.tray.time_format,
            &mut errs,
        );
        read_bool(
            entry(s, "color_icon"),
            "tray.color_icon",
            &mut cfg.tray.color_icon,
            &mut errs,
        );
    }

    if let Some(n_node) = entry(top, "notifications") {
        let s = section(
            n_node,
            "notifications",
            &[
                "idle_reminder",
                "long_running",
                "after_restart",
                "offline",
                "work_hours",
            ],
            &mut errs,
        );
        read_opt_duration(
            entry(s, "idle_reminder"),
            "notifications.idle_reminder",
            &mut cfg.notifications.idle_reminder,
            &mut errs,
        );
        read_opt_duration(
            entry(s, "long_running"),
            "notifications.long_running",
            &mut cfg.notifications.long_running,
            &mut errs,
        );
        read_bool(
            entry(s, "after_restart"),
            "notifications.after_restart",
            &mut cfg.notifications.after_restart,
            &mut errs,
        );
        read_bool(
            entry(s, "offline"),
            "notifications.offline",
            &mut cfg.notifications.offline,
            &mut errs,
        );
        if let Some(wh_node) = entry(s, "work_hours") {
            let w = section(
                wh_node,
                "notifications.work_hours",
                &["days", "from", "to"],
                &mut errs,
            );
            if let Some(n) = entry(w, "days") {
                match n.as_seq() {
                    Some(items) => {
                        let mut days = Vec::new();
                        for item in items {
                            match item.as_scalar().and_then(parse_weekday) {
                                Some(d) if !days.contains(&d) => days.push(d),
                                Some(_) => errs
                                    .push(item, "duplicate day in notifications.work_hours.days"),
                                None => errs
                                    .push(item, "days must be mon, tue, wed, thu, fri, sat or sun"),
                            }
                        }
                        cfg.notifications.work_hours.days = days;
                    }
                    None if n.is_null() => cfg.notifications.work_hours.days = Vec::new(),
                    None => errs.push(
                        n,
                        "notifications.work_hours.days must be a list such as [mon, tue]",
                    ),
                }
            }
            read_clock(
                entry(w, "from"),
                "notifications.work_hours.from",
                &mut cfg.notifications.work_hours.from,
                &mut errs,
            );
            read_clock(
                entry(w, "to"),
                "notifications.work_hours.to",
                &mut cfg.notifications.work_hours.to,
                &mut errs,
            );
            if cfg.notifications.work_hours.from >= cfg.notifications.work_hours.to {
                errs.push(
                    wh_node,
                    "notifications.work_hours.from must be earlier than to",
                );
            }
        }
    }

    if let Some(p_node) = entry(top, "popover") {
        let p = section(p_node, "popover", &["scroll"], &mut errs);
        if let Some(sc_node) = entry(p, "scroll") {
            let s = section(
                sc_node,
                "popover.scroll",
                &[
                    "confirm_with_hotkey",
                    "dwell_confirm",
                    "dwell_time",
                    "timeout",
                    "sensitivity",
                    "wrap_around",
                    "open_at_cursor",
                    "start_at",
                    "horizontal",
                ],
                &mut errs,
            );
            let sc = &mut cfg.popover.scroll;
            read_bool(
                entry(s, "confirm_with_hotkey"),
                "popover.scroll.confirm_with_hotkey",
                &mut sc.confirm_with_hotkey,
                &mut errs,
            );
            read_bool(
                entry(s, "dwell_confirm"),
                "popover.scroll.dwell_confirm",
                &mut sc.dwell_confirm,
                &mut errs,
            );
            read_duration(
                entry(s, "dwell_time"),
                "popover.scroll.dwell_time",
                &mut sc.dwell_time,
                &mut errs,
            );
            read_duration(
                entry(s, "timeout"),
                "popover.scroll.timeout",
                &mut sc.timeout,
                &mut errs,
            );
            if let Some(n) = entry(s, "sensitivity") {
                match n.as_f64() {
                    Some(v) if v > 0.0 && v <= 100.0 => sc.sensitivity = v,
                    _ => errs.push(
                        n,
                        "popover.scroll.sensitivity must be a number greater than 0",
                    ),
                }
            }
            read_bool(
                entry(s, "wrap_around"),
                "popover.scroll.wrap_around",
                &mut sc.wrap_around,
                &mut errs,
            );
            read_bool(
                entry(s, "open_at_cursor"),
                "popover.scroll.open_at_cursor",
                &mut sc.open_at_cursor,
                &mut errs,
            );
            read_enum(
                entry(s, "start_at"),
                "popover.scroll.start_at",
                &[("running", StartAt::Running), ("first", StartAt::First)],
                &mut sc.start_at,
                &mut errs,
            );
            read_bool(
                entry(s, "horizontal"),
                "popover.scroll.horizontal",
                &mut sc.horizontal,
                &mut errs,
            );
            if sc.dwell_time.is_zero() {
                if let Some(n) = entry(s, "dwell_time") {
                    errs.push(n, "popover.scroll.dwell_time must be longer than 0");
                }
            }
            if sc.timeout <= sc.dwell_time {
                let n = entry(s, "timeout")
                    .or(entry(s, "dwell_time"))
                    .unwrap_or(sc_node);
                errs.push(n, "popover.scroll.timeout must be longer than dwell_time");
            }
        }
    }

    if let Some(c_node) = entry(top, "cli") {
        let s = section(c_node, "cli", &["autostart_daemon"], &mut errs);
        read_bool(
            entry(s, "autostart_daemon"),
            "cli.autostart_daemon",
            &mut cfg.cli.autostart_daemon,
            &mut errs,
        );
    }

    if let Some(l_node) = entry(top, "log") {
        let s = section(l_node, "log", &["level"], &mut errs);
        read_enum(
            entry(s, "level"),
            "log.level",
            &[
                ("error", "error"),
                ("warn", "warn"),
                ("info", "info"),
                ("debug", "debug"),
                ("trace", "trace"),
            ],
            &mut "",
            &mut errs,
        );
        if let Some(level) = entry(s, "level").and_then(Node::as_scalar) {
            cfg.log.level = level.to_string();
        }
    }

    if let Some(o_node) = entry(top, "openaction") {
        let s = section(o_node, "openaction", &["toggle_running"], &mut errs);
        read_enum(
            entry(s, "toggle_running"),
            "openaction.toggle_running",
            &[("stop", ToggleRunning::Stop), ("none", ToggleRunning::None)],
            &mut cfg.openaction.toggle_running,
            &mut errs,
        );
    }

    if errs.0.is_empty() {
        Ok(cfg)
    } else {
        errs.0.sort_by_key(|e| (e.line, e.col));
        Err(errs.0)
    }
}

pub fn validate_server_url(url: &str) -> Result<(), String> {
    let u = url.trim();
    let rest = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
        .ok_or_else(|| format!("server.url \"{u}\" must start with http:// or https://"))?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err(format!("server.url \"{u}\" has no host"));
    }
    Ok(())
}

impl Config {
    /// The bucket ID with `{hostname}` replaced.
    pub fn bucket_id(&self, hostname: &str) -> String {
        self.server.bucket_id.replace("{hostname}", hostname)
    }

    /// AW `pulsetime` belonging to the heartbeat interval.
    pub fn pulsetime(&self) -> Duration {
        self.server.heartbeat_interval.mul_f64(1.5)
    }

    /// Favorites directories, resolved against the configuration directory.
    pub fn favorite_dirs(&self, config_dir: &Path) -> Vec<PathBuf> {
        self.favorites
            .dirs
            .iter()
            .map(|d| resolve_dir(d, config_dir))
            .collect()
    }
}

/// Resolves `~/…` and relative paths (relative to the configuration directory).
pub fn resolve_dir(d: &str, config_dir: &Path) -> PathBuf {
    if d == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = d.strip_prefix("~/").or_else(|| d.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    let p = PathBuf::from(d);
    if p.is_absolute() {
        p
    } else {
        config_dir.join(p)
    }
}

/// Result of loading `config.yaml` from disk.
#[derive(Debug, Clone)]
pub enum ConfigLoad {
    /// The file doesn't exist; defaults apply.
    Missing(Config),
    Ok(Config),
    Invalid(Vec<YamlError>),
    Unreadable(String),
}

pub fn load_config_file(path: &Path) -> ConfigLoad {
    match std::fs::read_to_string(path) {
        Ok(src) => match parse_config(&src) {
            Ok(c) => ConfigLoad::Ok(c),
            Err(e) => ConfigLoad::Invalid(e),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ConfigLoad::Missing(Config::default())
        }
        Err(e) => ConfigLoad::Unreadable(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_yaml_matches_defaults() {
        let parsed = parse_config(DEFAULT_CONFIG_YAML).unwrap();
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn empty_is_default() {
        assert_eq!(parse_config("").unwrap(), Config::default());
        assert_eq!(parse_config("# nothing\n").unwrap(), Config::default());
    }

    #[test]
    fn spec_example_parses() {
        let src = r#"
server:
  url: http://localhost:5600
  bucket_id: "aw-manual_{hostname}"
  heartbeat_interval: 60s
favorites:
  dirs:
    - favorites.d
hotkeys:
  popover: Ctrl+Alt+Space
  stop: Ctrl+Alt+S
tray:
  display: template
  template: "{title} · {elapsed}"
  elapsed_format: 1h 23m
  time_format: 24h
  color_icon: true
notifications:
  idle_reminder: 15m
  long_running: 3h
  work_hours:
    days: [mon, tue, wed, thu, fri]
    from: "08:00"
    to: "18:00"
popover:
  scroll:
    confirm_with_hotkey: true
    dwell_confirm: false
    dwell_time: 2s
    timeout: 10s
    sensitivity: 1.0
    wrap_around: false
    open_at_cursor: true
    start_at: running
openaction:
  toggle_running: stop
"#;
        let c = parse_config(src).unwrap();
        assert_eq!(c.server.heartbeat_interval, Duration::from_secs(60));
        assert_eq!(c.pulsetime(), Duration::from_secs(90));
        assert_eq!(c.bucket_id("box"), "aw-manual_box");
    }

    #[test]
    fn errors_with_lines() {
        let src = "server:\n  url: ftp://x\n  heartbet: 5s\ntray:\n  display: fancy\n";
        let errs = parse_config(src).unwrap_err();
        let lines: Vec<usize> = errs.iter().map(|e| e.line).collect();
        assert_eq!(lines, vec![2, 3, 5], "{errs:?}");
        assert!(errs[1].message.contains("heartbet"));
    }

    #[test]
    fn disabling_and_values() {
        let src = "notifications:\n  idle_reminder: false\n  long_running: 90m\nhotkeys:\n  resume:\n  favorites:\n    acme/dev: Ctrl+Alt+1\n";
        let c = parse_config(src).unwrap();
        assert_eq!(c.notifications.idle_reminder, None);
        assert_eq!(
            c.notifications.long_running,
            Some(Duration::from_secs(5400))
        );
        assert_eq!(c.hotkeys.resume, None);
        assert_eq!(
            c.hotkeys.favorites,
            vec![("acme/dev".to_string(), "Ctrl+Alt+1".to_string())]
        );
    }

    #[test]
    fn timeout_must_exceed_dwell() {
        let errs =
            parse_config("popover:\n  scroll:\n    dwell_time: 5s\n    timeout: 5s\n").unwrap_err();
        assert!(errs[0].message.contains("timeout"));
    }

    #[test]
    fn bad_template_and_hotkeys() {
        assert!(parse_config("tray:\n  template: \"{foo}\"\n").is_err());
        assert!(parse_config("hotkeys:\n  popover: Ctrl+Alt\n").is_err());
        assert!(parse_config("hotkeys:\n  popover: Mouse4\n  stop: mouse4\n").is_err());
        assert!(parse_config("hotkeys:\n  popover: Mouse4\n").is_ok());
        assert!(parse_config("hotkeys:\n  popover: Mouse2\n").is_err());
    }

    #[test]
    fn hotkey_validation() {
        assert!(validate_hotkey("Ctrl+Alt+Space").is_ok());
        assert!(validate_hotkey("CommandOrControl+Shift+T").is_ok());
        assert!(validate_hotkey("F13").is_ok());
        assert!(validate_hotkey("Ctrl+").is_err());
        assert!(validate_hotkey("Ctrl+A+B").is_err());
        assert!(validate_hotkey("A+Ctrl").is_err());
    }

    #[test]
    fn unknown_schema_version() {
        assert!(parse_config("schema_version: 2\n").is_err());
        assert!(parse_config("schema_version: 1\n").is_ok());
    }

    #[test]
    fn work_hours_validation() {
        assert!(parse_config(
            "notifications:\n  work_hours:\n    from: \"18:00\"\n    to: \"08:00\"\n"
        )
        .is_err());
        assert!(parse_config("notifications:\n  work_hours:\n    days: [mon, funday]\n").is_err());
        let c = parse_config("notifications:\n  work_hours:\n    days: []\n").unwrap();
        assert!(c.notifications.work_hours.days.is_empty());
    }

    #[test]
    fn resolve_dirs() {
        let base = Path::new("/cfg/aw-manual");
        assert_eq!(
            resolve_dir("favorites.d", base),
            PathBuf::from("/cfg/aw-manual/favorites.d")
        );
        assert_eq!(resolve_dir("/abs", base), PathBuf::from("/abs"));
        if let Some(home) = dirs::home_dir() {
            assert_eq!(resolve_dir("~/Sync/x", base), home.join("Sync/x"));
        }
    }

    #[test]
    fn non_mapping_root() {
        assert!(parse_config("- a\n- b\n").is_err());
        assert!(parse_config("server: 5\n").is_err());
    }
}
