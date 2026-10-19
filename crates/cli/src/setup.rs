//! `aw-manual devices setup`: interactive configuration of a hardware device.

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use aw_manual_core::config::{load_config_file, ConfigLoad, CONFIG_FILE, DEFAULT_CONFIG_YAML};
use aw_manual_core::devices::{
    self, discovery, setup::apply_to_config, DeviceAction, DeviceConfig, DeviceSelector, Driver,
};
use aw_manual_core::paths;

/// How long to wait for a sign of life from the device.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(20);

pub enum SetupError {
    /// Bad usage, e.g. unknown driver or no terminal.
    Invalid(String),
    Other(String),
}

fn invalid<T>(msg: impl Into<String>) -> Result<T, SetupError> {
    Err(SetupError::Invalid(msg.into()))
}

fn other(e: impl std::fmt::Display) -> SetupError {
    SetupError::Other(e.to_string())
}

/// Reads one line after printing `msg`. Fails on end of input.
fn ask(msg: &str) -> Result<String, SetupError> {
    print!("{msg}");
    std::io::stdout().flush().map_err(other)?;
    let mut line = String::new();
    if std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(other)?
        == 0
    {
        return Err(SetupError::Other("aborted (end of input)".into()));
    }
    Ok(line.trim().to_string())
}

fn confirm(msg: &str, default: bool) -> Result<bool, SetupError> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        match ask(&format!("{msg} {hint} "))?.to_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" | "j" | "ja" => return Ok(true),
            "n" | "no" | "nein" => return Ok(false),
            _ => println!("Please answer y or n."),
        }
    }
}

/// Lets the user pick one of `items` by number. `q` aborts.
fn choose<T>(title: &str, items: &[(String, T)]) -> Result<usize, SetupError> {
    println!("{title}");
    for (i, (label, _)) in items.iter().enumerate() {
        println!("  {}) {label}", i + 1);
    }
    loop {
        let answer = ask(&format!("Number (1-{}, q to quit): ", items.len()))?;
        if answer.eq_ignore_ascii_case("q") {
            return Err(SetupError::Other("aborted".into()));
        }
        match answer.parse::<usize>() {
            Ok(n) if (1..=items.len()).contains(&n) => return Ok(n - 1),
            _ => println!("Not a valid choice."),
        }
    }
}

pub fn run(driver_id: Option<String>) -> Result<(), SetupError> {
    if !std::io::stdin().is_terminal() {
        return invalid("`devices setup` is interactive and needs a terminal");
    }
    let drv = pick_driver(driver_id)?;
    println!("\nSetting up {}.\n", drv.title());

    let selector = pick_device(drv)?;
    let mut config = DeviceConfig::new(drv);
    config.enabled = true;
    config.selector = selector;

    verify(&config, drv)?;
    customize_bindings(&mut config, drv)?;

    let dir = paths::config_dir();
    let path = dir.join(CONFIG_FILE);
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_CONFIG_YAML.to_string(),
        Err(e) => return Err(other(format!("cannot read {}: {e}", path.display()))),
    };
    if let ConfigLoad::Ok(c) = load_config_file(&path) {
        if c.devices.iter().any(|d| d.driver == drv.id() && d.enabled)
            && !confirm(
                &format!(
                    "devices.{} is already enabled in {}. Replace it?",
                    drv.id(),
                    path.display()
                ),
                false,
            )?
        {
            return Err(SetupError::Other("aborted, nothing changed".into()));
        }
    }
    let updated = apply_to_config(&existing, &config)
        .map_err(|e| other(format!("cannot update {}: {e}", path.display())))?;
    write_atomic(&dir, &path, &updated).map_err(other)?;

    println!(
        "\nSaved {}. A running daemon applies it right away; otherwise start it with `aw-manual-daemon`.",
        path.display()
    );
    println!(
        "Check what the device sends any time with `aw-manual devices monitor {}`.",
        drv.id()
    );
    Ok(())
}

fn pick_driver(id: Option<String>) -> Result<&'static dyn Driver, SetupError> {
    let drivers = devices::drivers();
    if let Some(id) = id {
        return devices::driver(&id).map_or_else(
            || {
                invalid(format!(
                    "unknown driver \"{id}\" (available: {})",
                    drivers
                        .iter()
                        .map(|d| d.id())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            },
            Ok,
        );
    }
    if let [only] = drivers {
        return Ok(*only);
    }
    let items: Vec<(String, ())> = drivers
        .iter()
        .map(|d| (format!("{} ({})", d.title(), d.id()), ()))
        .collect();
    Ok(drivers[choose("Which kind of device?", &items)?])
}

/// Scans for devices (rescanning on request) and returns the selection that
/// identifies the chosen one.
fn pick_device(drv: &dyn Driver) -> Result<DeviceSelector, SetupError> {
    loop {
        let found =
            discovery::discover_midi(&discovery::Roots::default()).map_err(SetupError::Other)?;
        if found.is_empty() {
            println!(
                "No MIDI device is connected. Plug in the {} and wait a moment.",
                drv.title()
            );
            if !confirm("Scan again?", true)? {
                return Err(SetupError::Other("aborted".into()));
            }
            continue;
        }
        let items: Vec<(String, usize)> = found
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let usb = d
                    .usb_id
                    .as_ref()
                    .map_or(String::new(), |u| format!(", USB {u}"));
                let access = if d.readable { "" } else { " [not readable]" };
                (format!("{} ({}{usb}){access}", d.name, d.description), i)
            })
            .collect();
        let chosen = &found[choose("Connected devices:", &items)?];
        if !chosen.readable {
            println!(
                "You cannot read {}. Add your user to the \"audio\" group (then log in again) or fix the permissions of /dev/snd.",
                chosen.nodes[0].display()
            );
            if !confirm("Use it anyway?", false)? {
                continue;
            }
        }
        // Prefer the USB ID: it is stable and doesn't depend on the name.
        return Ok(match &chosen.usb_id {
            Some(id) => DeviceSelector {
                usb_id: Some(id.clone()),
                ..Default::default()
            },
            None => DeviceSelector {
                name: Some(chosen.name.clone()),
                ..Default::default()
            },
        });
    }
}

/// Waits for the first input of the device, so a wrong choice shows up now.
fn verify(config: &DeviceConfig, drv: &dyn Driver) -> Result<(), SetupError> {
    println!(
        "\nTest the connection: use the {} now (press it, turn it, …). Waiting up to {} s …",
        drv.title(),
        VERIFY_TIMEOUT.as_secs()
    );
    let stop = AtomicBool::new(false);
    let seen: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            devices::runner::run(config, &stop, |input| {
                let mut seen = seen.lock().unwrap();
                if seen.is_none() {
                    *seen = Some(input.to_string());
                    stop.store(true, Ordering::Relaxed);
                }
            })
        });
        let deadline = Instant::now() + VERIFY_TIMEOUT;
        while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        stop.store(true, Ordering::Relaxed);
    });
    match seen.into_inner().unwrap() {
        Some(input) => {
            println!("Received \"{input}\". The device works.");
            Ok(())
        }
        None => {
            println!("Nothing received.");
            if confirm("Save the configuration anyway?", false)? {
                Ok(())
            } else {
                Err(SetupError::Other("aborted, nothing changed".into()))
            }
        }
    }
}

/// Offers to change what the inputs do; Enter keeps the default.
fn customize_bindings(config: &mut DeviceConfig, drv: &dyn Driver) -> Result<(), SetupError> {
    println!("\nWhat the inputs do:");
    for i in drv.inputs() {
        let action = config
            .action(i.name)
            .map_or("none".to_string(), |a| a.to_string());
        println!("  {:<12} {:<10} ({})", i.name, action, i.description);
    }
    if !confirm("Change something?", false)? {
        return Ok(());
    }
    println!("Actions: popover, open, stop, resume, next, prev, confirm, back, cancel, favorite:<id>, none. Enter keeps the current action.");
    for i in drv.inputs() {
        let current = config.action(i.name).cloned();
        loop {
            let shown = current
                .as_ref()
                .map_or("none".to_string(), |a| a.to_string());
            let answer = ask(&format!("  {} [{shown}]: ", i.name))?;
            if answer.is_empty() {
                break;
            }
            match DeviceAction::parse(&answer) {
                Ok(action) => {
                    config.bindings.retain(|(n, _)| n != i.name);
                    if let Some(a) = action {
                        config.bindings.push((i.name.to_string(), a));
                    }
                    break;
                }
                Err(e) => println!("  {e}"),
            }
        }
    }
    Ok(())
}

fn write_atomic(dir: &Path, path: &Path, content: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}
