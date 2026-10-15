//! Whether the screen is locked, polled to detect unlocking.
//!
//! Each platform has one or more sources; the screen counts as locked if any
//! source says so (see [`locked`]). `aw-manual-daemon --check-lock` prints all
//! sources live, for debugging.
//!
//! - Linux: logind's `LockedHint` of the user's sessions (GNOME, KDE and
//!   lockers that report to logind), the `GetActive` method of the screen
//!   saver D-Bus services (freedesktop, GNOME, Cinnamon, MATE, Xfce), and
//!   running screen locker processes (hyprlock, swaylock, i3lock, …), which
//!   often report nothing.
//! - Windows: the input desktop can't be opened while the lock screen shows.
//! - macOS: `CGSSessionScreenIsLocked` in the current session dictionary.

/// The answer of one source: locked, not locked, or why it can't tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub source: String,
    pub value: Result<bool, String>,
}

impl Reading {
    fn new(source: impl Into<String>, value: Result<bool, String>) -> Reading {
        Reading {
            source: source.into(),
            value,
        }
    }
}

impl std::fmt::Display for Reading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.value {
            Ok(true) => write!(f, "{}: locked", self.source),
            Ok(false) => write!(f, "{}: not locked", self.source),
            Err(e) => write!(f, "{}: unavailable ({e})", self.source),
        }
    }
}

/// Locked if any source says so, unlocked if at least one source answered,
/// `None` if no source can tell.
pub fn locked(readings: &[Reading]) -> Option<bool> {
    let mut known = None;
    for r in readings {
        match r.value {
            Ok(true) => return Some(true),
            Ok(false) => known = Some(false),
            Err(_) => {}
        }
    }
    known
}

#[cfg(target_os = "linux")]
mod imp {
    use std::os::unix::fs::MetadataExt;

    use zbus::proxy::{Builder, CacheProperties};
    use zbus::zvariant::OwnedObjectPath;
    use zbus::{Connection, Proxy};

    use super::Reading;

    /// Screen saver services on the session bus: (name, object path).
    const SCREEN_SAVERS: &[(&str, &str)] = &[
        (
            "org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
        ),
        ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"),
        ("org.cinnamon.ScreenSaver", "/org/cinnamon/ScreenSaver"),
        ("org.mate.ScreenSaver", "/org/mate/ScreenSaver"),
        ("org.xfce.ScreenSaver", "/org/xfce/ScreenSaver"),
    ];

    /// Screen lockers that run only while the screen is locked (process
    /// names as in /proc/PID/comm, at most 15 characters).
    const LOCKERS: &[&str] = &[
        "hyprlock",
        "swaylock",
        "gtklock",
        "waylock",
        "i3lock",
        "xsecurelock",
        "slock",
        "physlock",
        "xlock",
        "kscreenlocker_g",
    ];

    pub struct Probe {
        system: Option<Connection>,
        session: Option<Connection>,
        uid: Option<u32>,
    }

    async fn proxy<'a>(
        conn: &Connection,
        dest: &'a str,
        path: &'a str,
        iface: &'a str,
    ) -> zbus::Result<Proxy<'a>> {
        Builder::new(conn)
            .destination(dest)?
            .path(path)?
            .interface(iface)?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    async fn has_owner(conn: &Connection, name: &str) -> zbus::Result<bool> {
        let dbus = zbus::fdo::DBusProxy::new(conn).await?;
        dbus.name_has_owner(name.try_into()?)
            .await
            .map_err(Into::into)
    }

    impl Probe {
        pub async fn new() -> Probe {
            Probe {
                system: Connection::system().await.ok(),
                session: Connection::session().await.ok(),
                uid: std::fs::metadata("/proc/self").ok().map(|m| m.uid()),
            }
        }

        pub async fn read(&mut self) -> Vec<Reading> {
            let mut out = Vec::new();
            self.logind(&mut out).await;
            self.screen_savers(&mut out).await;
            out.push(self.locker_processes());
            out
        }

        /// `LockedHint` of all sessions of this user.
        async fn logind(&self, out: &mut Vec<Reading>) {
            let Some(conn) = &self.system else {
                out.push(Reading::new("logind", Err("no system bus".into())));
                return;
            };
            let sessions = async {
                let manager = proxy(
                    conn,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                )
                .await?;
                manager
                    .call::<_, _, Vec<(String, u32, String, String, OwnedObjectPath)>>(
                        "ListSessions",
                        &(),
                    )
                    .await
            }
            .await;
            let sessions = match sessions {
                Ok(s) => s,
                Err(e) => {
                    out.push(Reading::new("logind", Err(e.to_string())));
                    return;
                }
            };
            let mine: Vec<_> = sessions
                .into_iter()
                .filter(|(_, uid, ..)| Some(*uid) == self.uid)
                .collect();
            if mine.is_empty() {
                out.push(Reading::new(
                    "logind",
                    Err("no session of this user".into()),
                ));
            }
            for (id, _, _, seat, path) in mine {
                let source = if seat.is_empty() {
                    format!("logind session {id} LockedHint")
                } else {
                    format!("logind session {id} ({seat}) LockedHint")
                };
                let value = async {
                    proxy(
                        conn,
                        "org.freedesktop.login1",
                        path.as_str(),
                        "org.freedesktop.login1.Session",
                    )
                    .await?
                    .get_property::<bool>("LockedHint")
                    .await
                }
                .await;
                out.push(Reading::new(source, value.map_err(|e| e.to_string())));
            }
        }

        /// `GetActive` of the screen saver services that are running.
        async fn screen_savers(&self, out: &mut Vec<Reading>) {
            let Some(conn) = &self.session else {
                out.push(Reading::new("screen saver", Err("no session bus".into())));
                return;
            };
            for (name, path) in SCREEN_SAVERS {
                // Calling a name without owner could start a service: check first.
                let value = match has_owner(conn, name).await {
                    Ok(false) => continue,
                    Err(e) => Err(e.to_string()),
                    Ok(true) => async {
                        proxy(conn, name, path, name)
                            .await?
                            .call::<_, _, bool>("GetActive", &())
                            .await
                    }
                    .await
                    .map_err(|e| e.to_string()),
                };
                out.push(Reading::new(format!("{name} GetActive"), value));
            }
        }

        /// A running screen locker of this user.
        fn locker_processes(&self) -> Reading {
            let source = "screen locker process";
            let entries = match std::fs::read_dir("/proc") {
                Ok(e) => e,
                Err(e) => return Reading::new(source, Err(e.to_string())),
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let is_pid = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()));
                if !is_pid {
                    continue;
                }
                let Ok(comm) = std::fs::read_to_string(path.join("comm")) else {
                    continue;
                };
                let comm = comm.trim();
                if !LOCKERS.contains(&comm) {
                    continue;
                }
                let uid = std::fs::metadata(&path).ok().map(|m| m.uid());
                if self.uid.is_none() || uid == self.uid {
                    return Reading::new(format!("{source} ({comm})"), Ok(true));
                }
            }
            Reading::new(source, Ok(false))
        }
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, OpenInputDesktop, DESKTOP_SWITCHDESKTOP,
    };

    use super::Reading;

    pub struct Probe;

    impl Probe {
        pub async fn new() -> Probe {
            Probe
        }

        pub async fn read(&mut self) -> Vec<Reading> {
            // SAFETY: plain Win32 calls; the handle is closed right away.
            let locked = unsafe {
                let desktop = OpenInputDesktop(0, 0, DESKTOP_SWITCHDESKTOP);
                if desktop.is_null() {
                    true
                } else {
                    CloseDesktop(desktop);
                    false
                }
            };
            vec![Reading::new("input desktop", Ok(locked))]
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::CFString;

    use super::Reading;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    }

    pub struct Probe;

    impl Probe {
        pub async fn new() -> Probe {
            Probe
        }

        pub async fn read(&mut self) -> Vec<Reading> {
            let source = "CGSSessionScreenIsLocked";
            // SAFETY: the dictionary is returned retained ("Copy" rule) and
            // released when `dict` is dropped.
            let value = unsafe {
                let raw = CGSessionCopyCurrentDictionary();
                if raw.is_null() {
                    Err("no session dictionary".to_string())
                } else {
                    let dict: CFDictionary<CFString, CFType> =
                        CFDictionary::wrap_under_create_rule(raw);
                    let key = CFString::from_static_string(source);
                    Ok(dict
                        .find(&key)
                        .and_then(|v| v.downcast::<CFBoolean>())
                        .is_some_and(bool::from))
                }
            };
            vec![Reading::new(source, value)]
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod imp {
    use super::Reading;

    pub struct Probe;

    impl Probe {
        pub async fn new() -> Probe {
            Probe
        }

        pub async fn read(&mut self) -> Vec<Reading> {
            vec![Reading::new("lock detection", Err("not supported".into()))]
        }
    }
}

pub use imp::Probe;

/// `aw-manual-daemon --check-lock`: prints the lock state and every source
/// whenever something changes, until Ctrl+C.
pub fn check_lock() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime");
    rt.block_on(async {
        println!(
            "Lock the screen, wait a few seconds, then unlock it.\n\
             \"locked\" must show while the screen is locked. Press Ctrl+C to quit.\n"
        );
        let mut probe = Probe::new().await;
        let mut last: Option<Vec<Reading>> = None;
        let mut iv = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            iv.tick().await;
            let readings = probe.read().await;
            if last.as_ref() == Some(&readings) {
                continue;
            }
            let state = match locked(&readings) {
                Some(true) => "LOCKED",
                Some(false) => "unlocked",
                None => "unknown (no source can tell)",
            };
            println!("{}  {state}", chrono::Local::now().format("%H:%M:%S"));
            for r in &readings {
                println!("    {r}");
            }
            last = Some(readings);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_locked_source_wins() {
        let r = |v: Result<bool, &str>| Reading::new("x", v.map_err(str::to_string));
        assert_eq!(locked(&[]), None);
        assert_eq!(locked(&[r(Err("no bus"))]), None);
        assert_eq!(locked(&[r(Err("no bus")), r(Ok(false))]), Some(false));
        assert_eq!(locked(&[r(Ok(false)), r(Ok(true))]), Some(true));
    }
}
