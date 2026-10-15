//! `aw-manual` – command line interface of the aw-manual daemon.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use aw_manual_core::config::{load_config_file, Config, ConfigLoad, CONFIG_FILE};
use aw_manual_core::engine::{AdhocActivity, ErrorCode, RunningInfo, Status, Target};
use aw_manual_core::favorites::{self, FileStatus, Kind};
use aw_manual_core::format::{format_clock, format_elapsed, Template, PATH_SEPARATOR};
use aw_manual_core::ipc::{self, ClientError, Endpoint};
use aw_manual_core::model::Origin;
use aw_manual_core::timeparse::{self, EndSpec, PointSpec};
use aw_manual_core::{display, paths};
use chrono::{Local, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};

const EXIT_CODES: &str = "\
Exit codes:
  0  success
  1  other error
  2  invalid command line
  3  the daemon is not running and could not be started
  4  unknown favorite ID
  5  invalid input (folder instead of leaf, invalid time range, …)
  6  not possible in the current state (nothing running, nothing to resume)
  7  configuration check failed
  8  not available (e.g. no popover when the daemon runs headless)";

mod exit {
    pub const OK: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const NO_DAEMON: u8 = 3;
    pub const NOT_FOUND: u8 = 4;
    pub const INVALID: u8 = 5;
    pub const STATE: u8 = 6;
    pub const CONFIG: u8 = 7;
    pub const UNAVAILABLE: u8 = 8;
}

#[derive(Parser)]
#[command(
    name = "aw-manual",
    version,
    about = "Manual activity tracking for ActivityWatch",
    after_help = EXIT_CODES
)]
struct Cli {
    /// Print machine-readable JSON (versioned schema).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start a favorite (leaf) or an ad-hoc activity.
    Start(StartArgs),
    /// Set a new note: the running activity ends and starts again with the note ("" removes it).
    Note { text: String },
    /// Stop the running activity.
    Stop,
    /// Start a favorite, or stop it if it is already running.
    Toggle {
        #[arg(value_name = "FAVORITE-ID")]
        favorite: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Resume the last stopped activity.
    Resume,
    /// Record a retroactive entry.
    Log(LogArgs),
    /// Show the running activity.
    Status {
        /// Template with {title} {path} {elapsed} {since} {note}; prints an empty line when nothing runs.
        #[arg(long)]
        format: Option<String>,
        /// Keep running and print a new line on every change (for status bars such as Waybar).
        #[arg(long)]
        follow: bool,
    },
    /// List all favorites in their configured order.
    List,
    /// Configuration commands.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Open the popover, or confirm the highlighted item if it is already open.
    Popover,
    /// Send a navigation command to the open popover.
    Nav {
        #[arg(value_enum)]
        command: NavArg,
    },
    /// Stream state changes as JSON lines.
    Watch,
    /// Open the settings window.
    Settings,
    /// Reload configuration and favorites.
    #[command(hide = true)]
    Reload,
    /// Quit the daemon.
    #[command(hide = true)]
    Quit,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Validate config.yaml and all favorites files.
    Check {
        /// Also warn about leaves without an explicit ID (and fail on warnings).
        #[arg(long)]
        strict: bool,
    },
    /// Print the configuration directory.
    Path,
}

#[derive(Args)]
struct StartArgs {
    /// ID of a favorite leaf, e.g. acme/website/development.
    #[arg(
        value_name = "FAVORITE-ID",
        required_unless_present = "adhoc",
        conflicts_with = "adhoc"
    )]
    favorite: Option<String>,
    /// Start an activity that is not defined in the configuration.
    #[arg(long, requires_all = ["adhoc_id", "title"])]
    adhoc: bool,
    /// ID of the ad-hoc activity.
    #[arg(long = "id", value_name = "ID", requires = "adhoc")]
    adhoc_id: Option<String>,
    /// Title of the ad-hoc activity.
    #[arg(long, requires = "adhoc")]
    title: Option<String>,
    /// Folder path above the ad-hoc activity, e.g. ACME/Website (the title is appended).
    #[arg(long, requires = "adhoc")]
    path: Option<String>,
    #[arg(long, value_enum, requires = "adhoc", default_value = "activity")]
    kind: KindArg,
    #[arg(long)]
    note: Option<String>,
}

#[derive(Args)]
#[command(after_help = "\
Times: 9:30, 14:00, 2:30pm, yesterday 16:00, 2026-10-09 09:30, -30m (30 minutes ago), now.
Ranges for --range: -30m, 10:00-11:15, since 9:30, 10:00 +45m, yesterday 16:00-17:30.")]
struct LogArgs {
    #[arg(value_name = "FAVORITE-ID")]
    favorite: String,
    /// Start of the entry.
    #[arg(long, allow_hyphen_values = true, requires = "end_spec", conflicts_with_all = ["last", "range"])]
    from: Option<String>,
    /// End of the entry.
    #[arg(long, allow_hyphen_values = true, group = "end_spec")]
    to: Option<String>,
    /// Duration of the entry (with --from).
    #[arg(long, group = "end_spec")]
    duration: Option<String>,
    /// The last N minutes/hours until now, e.g. 30m.
    #[arg(long, allow_hyphen_values = true, conflicts_with_all = ["range", "to", "duration"])]
    last: Option<String>,
    /// A range expression as in the popover, e.g. "10:00-11:15".
    #[arg(long, allow_hyphen_values = true, conflicts_with_all = ["to", "duration"])]
    range: Option<String>,
    #[arg(long)]
    note: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    Activity,
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
enum NavArg {
    Next,
    Prev,
    Confirm,
    Back,
    Cancel,
}

impl NavArg {
    fn as_str(self) -> &'static str {
        match self {
            NavArg::Next => "next",
            NavArg::Prev => "prev",
            NavArg::Confirm => "confirm",
            NavArg::Back => "back",
            NavArg::Cancel => "cancel",
        }
    }
}

/// An error with its exit code.
struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Failure {
            code,
            message: message.into(),
        }
    }
}

impl From<ClientError> for Failure {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::NotRunning(_) => Failure::new(exit::NO_DAEMON, e.to_string()),
            ClientError::Io(_) => Failure::new(exit::ERROR, e.to_string()),
            ClientError::Remote(r) => {
                let code = match r.code {
                    ErrorCode::NotFound => exit::NOT_FOUND,
                    ErrorCode::IsFolder | ErrorCode::Invalid => exit::INVALID,
                    ErrorCode::NotRunning | ErrorCode::NothingToResume => exit::STATE,
                    ErrorCode::Unavailable => exit::UNAVAILABLE,
                    ErrorCode::Internal | ErrorCode::BadRequest => exit::ERROR,
                };
                Failure::new(code, r.message)
            }
        }
    }
}

type Result<T> = std::result::Result<T, Failure>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::from(exit::OK),
        Err(f) => {
            if !f.message.is_empty() {
                eprintln!("aw-manual: {}", f.message);
            }
            ExitCode::from(f.code)
        }
    }
}

/// Loads config.yaml for local formatting and daemon start (defaults if missing or invalid).
fn local_config() -> Config {
    match load_config_file(&paths::config_dir().join(CONFIG_FILE)) {
        ConfigLoad::Ok(c) | ConfigLoad::Missing(c) => c,
        _ => Config::default(),
    }
}

fn endpoint() -> Result<Endpoint> {
    ipc::endpoint().map_err(|e| {
        Failure::new(
            exit::ERROR,
            format!("cannot determine the daemon socket: {e}"),
        )
    })
}

const TIMEOUT: Duration = Duration::from_secs(10);

/// Calls the daemon, starting it first if necessary (and allowed).
fn request(method: &str, params: Value) -> Result<Value> {
    let ep = endpoint()?;
    match ipc::call(&ep, method, params.clone(), TIMEOUT) {
        Err(ClientError::NotRunning(e)) => {
            if !local_config().cli.autostart_daemon {
                return Err(Failure::new(
                    exit::NO_DAEMON,
                    format!("the aw-manual daemon is not running ({e}); start it or enable cli.autostart_daemon"),
                ));
            }
            start_daemon(&ep)?;
            Ok(ipc::call(&ep, method, params, TIMEOUT)?)
        }
        r => Ok(r?),
    }
}

fn daemon_binary() -> Option<PathBuf> {
    let name = format!("aw-manual-daemon{}", std::env::consts::EXE_SUFFIX);
    if let Some(p) = std::env::var_os("AW_MANUAL_DAEMON").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.canonicalize().unwrap_or(exe);
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(&name))
        .find(|p| p.is_file())
}

fn start_daemon(ep: &Endpoint) -> Result<()> {
    let bin = daemon_binary().ok_or_else(|| {
        Failure::new(
            exit::NO_DAEMON,
            "the aw-manual daemon is not running and aw-manual-daemon was not found",
        )
    })?;
    let mut cmd = std::process::Command::new(&bin);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe; it detaches the daemon from the terminal session.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map_err(|e| {
        Failure::new(
            exit::NO_DAEMON,
            format!("cannot start {}: {e}", bin.display()),
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        if ipc::call(ep, "ping", Value::Null, Duration::from_secs(2)).is_ok() {
            return Ok(());
        }
    }
    Err(Failure::new(
        exit::NO_DAEMON,
        format!("started {} but it did not respond", bin.display()),
    ))
}

fn out(s: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{s}")
        .and_then(|_| stdout.flush())
        .map_err(|_| Failure::new(exit::OK, ""))
}

fn print_json(v: &Value) -> Result<()> {
    out(&serde_json::to_string(v).expect("JSON serializes"))
}

fn parse_status(v: &Value) -> Result<Status> {
    serde_json::from_value(v.clone()).map_err(|e| {
        Failure::new(
            exit::ERROR,
            format!("unexpected status from the daemon: {e}"),
        )
    })
}

fn describe(r: &RunningInfo) -> String {
    r.path.join(PATH_SEPARATOR)
}

/// Human-readable result of a state-changing command.
fn report(result: &Value, json: bool) -> Result<()> {
    if json {
        return print_json(result);
    }
    let change = result["change"].as_str().unwrap_or_default();
    let status = parse_status(&result["status"])?;
    let running = status.running.as_ref().map(describe).unwrap_or_default();
    let msg = match change {
        "started" => format!("Started {running}"),
        "switched" => format!("Switched to {running}"),
        "note_changed" => format!(
            "New note for {running}: {}",
            status
                .running
                .as_ref()
                .and_then(|r| r.note.clone())
                .unwrap_or_else(|| "(none)".into())
        ),
        "stopped" => match &status.last_stopped {
            Some(s) => format!("Stopped {}", s.path.join(PATH_SEPARATOR)),
            None => "Stopped".into(),
        },
        "logged" => "Logged".into(),
        "unchanged" => format!("Already running: {running}"),
        other => other.to_string(),
    };
    out(&msg)
}

fn note_param(note: &Option<String>) -> Value {
    note.as_ref().map_or(Value::Null, |n| json!(n))
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    match cli.cmd {
        Cmd::Start(a) => {
            let target = match (a.adhoc, a.favorite) {
                (false, Some(f)) => Target::Favorite(f),
                _ => Target::Adhoc(AdhocActivity {
                    id: a.adhoc_id.unwrap_or_default(),
                    title: a.title.unwrap_or_default(),
                    path: a
                        .path
                        .map(|p| {
                            p.split('/')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    kind: match a.kind {
                        KindArg::Activity => Kind::Activity,
                        KindArg::Status => Kind::Status,
                    },
                    origin: Origin::Adhoc,
                    color: None,
                    icon: None,
                }),
            };
            let r = request(
                "start",
                json!({"target": target, "note": note_param(&a.note), "source": "cli"}),
            )?;
            report(&r, json)
        }
        Cmd::Note { text } => {
            let r = request("note", json!({"note": text, "source": "cli"}))?;
            report(&r, json)
        }
        Cmd::Stop => {
            let r = request("stop", json!({"source": "cli"}))?;
            report(&r, json)
        }
        Cmd::Toggle { favorite, note } => {
            let r = request(
                "toggle",
                json!({"target": Target::Favorite(favorite), "note": note_param(&note), "source": "cli"}),
            )?;
            report(&r, json)
        }
        Cmd::Resume => {
            let r = request("resume", json!({"source": "cli"}))?;
            report(&r, json)
        }
        Cmd::Log(a) => log(a, json),
        Cmd::Status { format, follow } => status(format, follow, json),
        Cmd::List => list(json),
        Cmd::Config {
            cmd: ConfigCmd::Check { strict },
        } => config_check(strict, json),
        Cmd::Config {
            cmd: ConfigCmd::Path,
        } => out(&paths::config_dir().display().to_string()),
        Cmd::Popover => {
            let r = request("popover", json!({"source": "cli"}))?;
            if json {
                print_json(&r)?;
            }
            Ok(())
        }
        Cmd::Nav { command } => {
            let r = request("nav", json!({"command": command.as_str()}))?;
            if json {
                print_json(&r)?;
            }
            Ok(())
        }
        Cmd::Watch => watch_loop(|v| print_json(&v)),
        Cmd::Settings => request("settings", Value::Null).map(|_| ()),
        Cmd::Reload => {
            let r = request("reload", Value::Null)?;
            if json {
                print_json(&r)?;
            }
            Ok(())
        }
        Cmd::Quit => {
            let ep = endpoint()?;
            match ipc::call(&ep, "quit", Value::Null, TIMEOUT) {
                Ok(_) | Err(ClientError::NotRunning(_)) => Ok(()),
                Err(e) => Err(e.into()),
            }
        }
    }
}

fn log(a: LogArgs, json: bool) -> Result<()> {
    let invalid = |e: String| Failure::new(exit::INVALID, e);
    let (start, end) = if let Some(last) = &a.last {
        let d = aw_manual_core::duration::parse_duration(last.trim_start_matches('-'))
            .map_err(invalid)?;
        (PointSpec::Ago(d), EndSpec::Point(PointSpec::Now))
    } else if let Some(range) = &a.range {
        timeparse::parse_range_spec(range).map_err(invalid)?
    } else if let Some(from) = &a.from {
        let start = timeparse::parse_point(from).map_err(invalid)?;
        let end = match (&a.to, &a.duration) {
            (Some(to), None) => EndSpec::Point(timeparse::parse_point(to).map_err(invalid)?),
            (None, Some(d)) => {
                EndSpec::Duration(aw_manual_core::duration::parse_duration(d).map_err(invalid)?)
            }
            _ => return Err(invalid("use --from with either --to or --duration".into())),
        };
        (start, end)
    } else {
        return Err(Failure::new(
            2,
            "specify the time with --from/--to, --from/--duration, --last or --range",
        ));
    };
    let (start, end) = timeparse::resolve_range(&start, &end, &Local::now()).map_err(invalid)?;
    let r = request(
        "log",
        json!({
            "target": Target::Favorite(a.favorite.clone()),
            "start": start,
            "end": end,
            "note": note_param(&a.note),
            "source": "cli",
        }),
    )?;
    if json {
        return print_json(&r);
    }
    let fmt = local_config().tray.time_format;
    out(&format!(
        "Logged {} {}–{} ({})",
        a.favorite,
        format_clock(&start.with_timezone(&Local), fmt),
        format_clock(&end.with_timezone(&Local), fmt),
        format_elapsed(
            (end - start).num_seconds(),
            local_config().tray.elapsed_format
        )
    ))
}

fn render_status(st: &Status, template: Option<&Template>, cfg: &Config) -> String {
    let now = Utc::now();
    if let Some(t) = template {
        return match &st.running {
            Some(r) => t.render(&display::vars(&cfg.tray, r, now)),
            None => String::new(),
        };
    }
    let mut lines = Vec::new();
    match &st.running {
        Some(r) => {
            lines.push(format!("● {}", describe(r)));
            let mut l = format!(
                "  since {} ({})",
                format_clock(&r.start.with_timezone(&Local), cfg.tray.time_format),
                format_elapsed((now - r.start).num_seconds(), cfg.tray.elapsed_format)
            );
            if r.kind == Kind::Status {
                l.push_str(" · status");
            }
            lines.push(l);
            if let Some(n) = &r.note {
                lines.push(format!("  note: {n}"));
            }
        }
        None => {
            lines.push("No activity running.".into());
            if let Some(s) = &st.last_stopped {
                lines.push(format!(
                    "  last: {} (ended {})",
                    s.path.join(PATH_SEPARATOR),
                    format_clock(&s.ended.with_timezone(&Local), cfg.tray.time_format)
                ));
            }
        }
    }
    match st.server.online {
        Some(false) => lines.push(format!(
            "aw-server unreachable ({}), {} entries pending",
            st.server.url, st.server.pending
        )),
        _ if st.server.pending > 0 => lines.push(format!("{} entries pending", st.server.pending)),
        _ => {}
    }
    if st.server.failed > 0 {
        lines.push(format!(
            "{} entries were rejected by aw-server (see the log)",
            st.server.failed
        ));
    }
    lines.join("\n")
}

fn status(format: Option<String>, follow: bool, json: bool) -> Result<()> {
    let cfg = local_config();
    let template = match &format {
        Some(f) => Some(Template::parse(f).map_err(|e| Failure::new(exit::INVALID, e))?),
        None => None,
    };
    if !follow {
        let v = request("status", Value::Null)?;
        if json {
            return print_json(&v);
        }
        return out(&render_status(&parse_status(&v)?, template.as_ref(), &cfg));
    }
    if json {
        return watch_loop(|v| print_json(&v));
    }

    // Follow: re-render on every change and when the elapsed time changes.
    let seconds = template.as_ref().is_some_and(|t| t.uses("elapsed"))
        && cfg.tray.elapsed_format.has_seconds()
        || template.is_none() && cfg.tray.elapsed_format.has_seconds();
    let (tx, rx) = std::sync::mpsc::channel::<Option<Value>>();
    std::thread::spawn(move || {
        let _ = watch_loop(|v| tx.send(Some(v)).map_err(|_| Failure::new(exit::OK, "")));
    });
    let mut current: Option<Status> = None;
    let mut last_output: Option<String> = None;
    loop {
        // Below one minute, "1h 23m" shows seconds ("42s") as well.
        let first_minute = current
            .as_ref()
            .and_then(|s| s.running.as_ref())
            .is_some_and(|r| (Utc::now() - r.start).num_seconds() < 60);
        let wait = if seconds || first_minute {
            Duration::from_millis(1000 - (Utc::now().timestamp_subsec_millis() as u64).min(999))
        } else {
            Duration::from_secs(60 - (Utc::now().timestamp() % 60) as u64)
        };
        match rx.recv_timeout(wait) {
            Ok(Some(v)) => current = Some(parse_status(&v)?),
            Ok(None) => current = None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let text = match &current {
            Some(st) => render_status(st, template.as_ref(), &cfg),
            None if template.is_some() => String::new(),
            None => "The aw-manual daemon is not running.".into(),
        };
        if last_output.as_ref() != Some(&text) {
            out(&text)?;
            last_output = Some(text);
        }
    }
}

/// Streams status values; reconnects if the daemon restarts.
fn watch_loop(mut f: impl FnMut(Value) -> Result<()>) -> Result<()> {
    let ep = endpoint()?;
    let mut first = true;
    loop {
        if first {
            // Starts the daemon if needed.
            request("ping", Value::Null)?;
            first = false;
        }
        if let Ok(mut c) = ipc::Client::connect(&ep) {
            {
                let mut err = None;
                let r = c.watch(|v| match f(v) {
                    Ok(()) => true,
                    Err(e) => {
                        err = Some(e);
                        false
                    }
                });
                if let Some(e) = err {
                    return Err(e);
                }
                if let Err(e) = r {
                    if matches!(e, ClientError::Remote(_)) {
                        return Err(e.into());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn list(json: bool) -> Result<()> {
    let v = request("list", Value::Null)?;
    if json {
        return print_json(&v);
    }
    fn walk(nodes: &Value, depth: usize, lines: &mut Vec<(String, String)>) {
        for n in nodes.as_array().into_iter().flatten() {
            let title = n["title"].as_str().unwrap_or_default();
            let folder = n["type"] == "folder";
            let mut label = format!(
                "{}{}{}",
                "  ".repeat(depth),
                title,
                if folder { "/" } else { "" }
            );
            if n["kind"] == "status" {
                label.push_str(" [status]");
            }
            lines.push((
                label,
                if folder {
                    String::new()
                } else {
                    n["id"].as_str().unwrap_or_default().to_string()
                },
            ));
            walk(&n["children"], depth + 1, lines);
        }
    }
    let mut lines = Vec::new();
    walk(&v["favorites"], 0, &mut lines);
    if lines.is_empty() {
        out("No favorites defined. Add YAML files to the favorites folder (see `aw-manual config path`).")?;
    }
    let width = lines
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    for (label, id) in &lines {
        if id.is_empty() {
            out(label)?;
        } else {
            out(&format!(
                "{label}{}  {id}",
                " ".repeat(width - label.chars().count())
            ))?;
        }
    }
    for f in v["files"].as_array().into_iter().flatten() {
        if f["status"] != "ok" {
            for e in f["errors"].as_array().into_iter().flatten() {
                eprintln!(
                    "warning: {}{}: {}{}",
                    f["path"].as_str().unwrap_or_default(),
                    e["line"]
                        .as_u64()
                        .map(|l| format!(":{l}"))
                        .unwrap_or_default(),
                    e["message"].as_str().unwrap_or_default(),
                    if f["status"] == "last_valid" {
                        " (the last valid version is used)"
                    } else {
                        ""
                    }
                );
            }
        }
    }
    Ok(())
}

fn config_check(strict: bool, json: bool) -> Result<()> {
    let dir = paths::config_dir();
    let cfg_path = dir.join(CONFIG_FILE);
    let (config, config_json, mut ok) = match load_config_file(&cfg_path) {
        ConfigLoad::Missing(c) => (
            c,
            json!({"path": cfg_path, "state": "missing", "errors": []}),
            true,
        ),
        ConfigLoad::Ok(c) => (
            c,
            json!({"path": cfg_path, "state": "ok", "errors": []}),
            true,
        ),
        ConfigLoad::Invalid(errs) => {
            let errors: Vec<Value> = errs
                .iter()
                .map(|e| json!({"line": e.line, "message": e.message}))
                .collect();
            (
                Config::default(),
                json!({"path": cfg_path, "state": "invalid", "errors": errors}),
                false,
            )
        }
        ConfigLoad::Unreadable(e) => (
            Config::default(),
            json!({"path": cfg_path, "state": "invalid", "errors": [{"line": null, "message": e}]}),
            false,
        ),
    };
    let load = favorites::load_favorites(&config.favorite_dirs(&dir), None);
    ok &= !load.has_errors();
    let mut warnings: Vec<String> = load.warnings.clone();
    for d in config.favorite_dirs(&dir) {
        if !d.is_dir() {
            warnings.push(format!("favorites folder {} does not exist", d.display()));
        }
    }
    if strict {
        for f in load.tree.all() {
            if f.is_leaf() && !f.explicit_id {
                warnings.push(format!(
                    "{}:{}: leaf \"{}\" has no explicit ID (its ID \"{}\" changes when the title changes; set _id)",
                    f.file.display(),
                    f.line,
                    f.title,
                    f.id
                ));
            }
        }
        ok &= warnings.is_empty();
    }

    if json {
        print_json(&json!({
            "schema_version": 1,
            "ok": ok,
            "config": config_json,
            "files": load.files,
            "warnings": warnings,
            "favorites": load.tree.leaves().len(),
        }))?;
    } else {
        match config_json["state"].as_str() {
            Some("missing") => out(&format!(
                "{}: not found, defaults apply",
                cfg_path.display()
            ))?,
            Some("ok") => out(&format!("{}: ok", cfg_path.display()))?,
            _ => {
                for e in config_json["errors"].as_array().into_iter().flatten() {
                    out(&format!(
                        "{}{}: {}",
                        cfg_path.display(),
                        e["line"]
                            .as_u64()
                            .map(|l| format!(":{l}"))
                            .unwrap_or_default(),
                        e["message"].as_str().unwrap_or_default()
                    ))?;
                }
            }
        }
        for f in &load.files {
            match &f.status {
                FileStatus::Ok => out(&format!(
                    "{}: ok{}{}",
                    f.path.display(),
                    f.notation
                        .map(|n| format!(
                            " ({})",
                            if n == favorites::Notation::Compact {
                                "compact"
                            } else {
                                "explicit"
                            }
                        ))
                        .unwrap_or_default(),
                    if f.generated { ", generated" } else { "" }
                ))?,
                FileStatus::Invalid { errors } | FileStatus::LastValid { errors } => {
                    for e in errors {
                        out(&format!(
                            "{}{}: {}",
                            f.path.display(),
                            e.line.map(|l| format!(":{l}")).unwrap_or_default(),
                            e.message
                        ))?;
                    }
                }
            }
        }
        for w in &warnings {
            out(&format!("warning: {w}"))?;
        }
        out(&format!(
            "{} startable favorites; {}",
            load.tree.leaves().len(),
            if ok {
                "configuration is valid"
            } else if strict
                && !warnings.is_empty()
                && config_json["state"] != "invalid"
                && !load.has_errors()
            {
                "configuration has warnings (--strict)"
            } else {
                "configuration has errors"
            }
        ))?;
    }
    if ok {
        Ok(())
    } else {
        Err(Failure::new(exit::CONFIG, ""))
    }
}
