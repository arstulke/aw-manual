//! Global mouse-button hotkeys (e.g. `Mouse4` on a side button).
//!
//! Platform backends:
//! - Linux/X11: passive button grab on the root window (the button is consumed)
//! - Windows: low-level mouse hook (the button is consumed)
//! - macOS: Quartz event tap (needs the Accessibility permission; the button is consumed)
//!
//! On Wayland, applications cannot capture global input; bind the button in
//! the compositor to `aw-manual popover` instead.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

/// State shared with the platform hook thread.
pub struct Shared {
    /// Configured buttons, numbered as in the configuration (`Mouse4` = 4).
    buttons: Mutex<HashSet<u8>>,
    /// Incremented whenever `buttons` changes.
    generation: AtomicU64,
    on_press: Box<dyn Fn(u8) + Send + Sync>,
}

impl Shared {
    pub fn buttons(&self) -> HashSet<u8> {
        self.buttons.lock().unwrap().clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn wants(&self, button: u8) -> bool {
        self.buttons.lock().unwrap().contains(&button)
    }

    pub fn pressed(&self, button: u8) {
        (self.on_press)(button)
    }
}

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

/// Parses `Mouse4` … `Mouse9`.
pub fn parse_button(hotkey: &str) -> Option<u8> {
    let n = hotkey
        .trim()
        .strip_prefix("Mouse")
        .or_else(|| hotkey.trim().strip_prefix("mouse"))?;
    n.parse::<u8>().ok().filter(|n| (4..=9).contains(n))
}

/// Sets the buttons to capture. The hook is started on first use (so the
/// macOS permission prompt only appears for users of mouse hotkeys).
pub fn set_buttons(
    buttons: HashSet<u8>,
    on_press: impl Fn(u8) + Send + Sync + 'static,
) -> Result<(), String> {
    if buttons.is_empty() && SHARED.get().is_none() {
        return Ok(());
    }
    if let Some(n) = buttons.iter().find(|n| !supported(**n)) {
        return Err(format!("Mouse{n} is not supported on this platform"));
    }
    let mut started_now = false;
    let shared = SHARED.get_or_init(|| {
        started_now = true;
        Arc::new(Shared {
            buttons: Mutex::new(HashSet::new()),
            generation: AtomicU64::new(0),
            on_press: Box::new(on_press),
        })
    });
    *shared.buttons.lock().unwrap() = buttons;
    shared.generation.fetch_add(1, Ordering::SeqCst);
    if started_now {
        start(shared.clone())?;
    }
    Ok(())
}

fn supported(n: u8) -> bool {
    if cfg!(windows) {
        matches!(n, 4 | 5)
    } else {
        (4..=9).contains(&n)
    }
}

#[cfg(target_os = "linux")]
fn start(shared: Arc<Shared>) -> Result<(), String> {
    if crate::popover::is_wayland() {
        return Err("mouse buttons cannot be captured on Wayland; bind the button in your compositor to `aw-manual popover`".into());
    }
    x11::start(shared)
}

#[cfg(target_os = "macos")]
fn start(shared: Arc<Shared>) -> Result<(), String> {
    macos::start(shared)
}

#[cfg(windows)]
fn start(shared: Arc<Shared>) -> Result<(), String> {
    windows::start(shared)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn start(_shared: Arc<Shared>) -> Result<(), String> {
    Err("mouse buttons are not supported on this platform".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse() {
        assert_eq!(super::parse_button("Mouse4"), Some(4));
        assert_eq!(super::parse_button("mouse9"), Some(9));
        assert_eq!(super::parse_button("Mouse3"), None);
        assert_eq!(super::parse_button("Ctrl+A"), None);
    }
}
