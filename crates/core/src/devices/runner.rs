//! Reads a device and reports its inputs; reconnects when it is unplugged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{discovery, driver, DeviceConfig, Gestures, Transport};

/// How often a missing or failing device is retried.
const RETRY: Duration = Duration::from_secs(3);

/// Runs until `stop` is set. Calls `on_input` with the name of every input
/// (`press`, `turn_left`, …); the caller maps it to an action. Blocks the
/// calling thread, so run it on its own thread.
pub fn run(config: &DeviceConfig, stop: &AtomicBool, mut on_input: impl FnMut(&str)) {
    let Some(drv) = driver(&config.driver) else {
        tracing::error!("unknown device driver \"{}\"", config.driver);
        return;
    };
    let mut last_problem: Option<String> = None;
    while !stop.load(Ordering::Relaxed) {
        match connect_and_read(config, stop, &mut on_input) {
            Ok(()) => return,
            Err(problem) => {
                // Log each distinct problem once, not every retry.
                if last_problem.as_ref() != Some(&problem) {
                    tracing::info!(
                        "{}: {problem}, retrying every {}s",
                        drv.title(),
                        RETRY.as_secs()
                    );
                    last_problem = Some(problem);
                }
            }
        }
        sleep_unless_stopped(RETRY, stop);
    }
}

fn sleep_unless_stopped(total: Duration, stop: &AtomicBool) {
    let end = Instant::now() + total;
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn connect_and_read(
    config: &DeviceConfig,
    stop: &AtomicBool,
    on_input: &mut impl FnMut(&str),
) -> Result<(), String> {
    let drv = driver(&config.driver).ok_or("unknown driver")?;
    let node = match drv.transport() {
        Transport::Midi => {
            let devices = discovery::discover_midi(&discovery::Roots::default())?;
            discovery::resolve(&config.selector, &devices).map_err(|e| e.to_string())?
        }
    };
    let mut decoder = drv.decoder();
    let mut gestures = Gestures::new(config.long_press);
    tracing::info!("{}: reading {}", drv.title(), node.display());
    let result = read_bytes(&node, stop, |bytes| {
        for signal in decoder.feed(bytes) {
            if let Some(input) = gestures.handle(signal, Instant::now()) {
                tracing::debug!("{}: {input}", drv.title());
                on_input(&input);
            }
        }
    });
    match result {
        Ok(()) => Ok(()),
        Err(e) => Err(describe_io_error(&node, &e)),
    }
}

fn describe_io_error(node: &std::path::Path, e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => format!(
            "cannot read {} (permission denied; is your user in the \"audio\" group?)",
            node.display()
        ),
        std::io::ErrorKind::NotFound => format!("{} does not exist (unplugged?)", node.display()),
        _ => format!("{}: {e}", node.display()),
    }
}

/// Reads `path` until `stop` is set (`Ok`) or the device fails or disappears
/// (`Err`). Wakes up regularly to check `stop`.
#[cfg(target_os = "linux")]
fn read_bytes(
    path: &std::path::Path,
    stop: &AtomicBool,
    mut on_bytes: impl FnMut(&[u8]),
) -> std::io::Result<()> {
    use std::io::Read;
    use std::os::fd::AsRawFd;

    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 256];
    while !stop.load(Ordering::Relaxed) {
        let mut pfd = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is a valid pollfd and the count is 1.
        let n = unsafe { libc::poll(&mut pfd, 1, 100) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            continue;
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "device disconnected",
            ));
        }
        match file.read(&mut buf) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "device disconnected",
                ))
            }
            Ok(n) => on_bytes(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn read_bytes(
    _path: &std::path::Path,
    _stop: &AtomicBool,
    _on_bytes: impl FnMut(&[u8]),
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "hardware devices are only supported on Linux",
    ))
}
