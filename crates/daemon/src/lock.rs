//! Whether the screen is locked, polled to detect unlocking.
//!
//! - Linux: logind's `LockedHint` of the session, and the `GetActive` method
//!   of the freedesktop / GNOME screen saver interfaces (session bus).
//! - Windows: the input desktop can't be opened while the lock screen shows.
//! - macOS: `CGSSessionScreenIsLocked` in the current session dictionary.

#[cfg(target_os = "linux")]
mod imp {
    use zbus::proxy::{Builder, CacheProperties};
    use zbus::{Connection, Proxy};

    pub struct Probe {
        system: Option<Connection>,
        session: Option<Connection>,
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

    impl Probe {
        pub async fn new() -> Probe {
            Probe {
                system: Connection::system().await.ok(),
                session: Connection::session().await.ok(),
            }
        }

        /// `None` if no source answers (unknown).
        pub async fn is_locked(&mut self) -> Option<bool> {
            let mut known = None;
            if let Some(c) = &self.system {
                if let Ok(p) = proxy(
                    c,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1/session/auto",
                    "org.freedesktop.login1.Session",
                )
                .await
                {
                    match p.get_property::<bool>("LockedHint").await {
                        Ok(true) => return Some(true),
                        Ok(false) => known = Some(false),
                        Err(_) => {}
                    }
                }
            }
            if let Some(c) = &self.session {
                for (dest, path) in [
                    (
                        "org.freedesktop.ScreenSaver",
                        "/org/freedesktop/ScreenSaver",
                    ),
                    ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"),
                ] {
                    let Ok(p) = proxy(c, dest, path, dest).await else {
                        continue;
                    };
                    match p.call::<_, _, bool>("GetActive", &()).await {
                        Ok(true) => return Some(true),
                        Ok(false) => known = Some(false),
                        Err(_) => {}
                    }
                }
            }
            known
        }
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, OpenInputDesktop, DESKTOP_SWITCHDESKTOP,
    };

    pub struct Probe;

    impl Probe {
        pub async fn new() -> Probe {
            Probe
        }

        pub async fn is_locked(&mut self) -> Option<bool> {
            // SAFETY: plain Win32 calls; the handle is closed right away.
            unsafe {
                let desktop = OpenInputDesktop(0, 0, DESKTOP_SWITCHDESKTOP);
                if desktop.is_null() {
                    Some(true)
                } else {
                    CloseDesktop(desktop);
                    Some(false)
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::CFString;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    }

    pub struct Probe;

    impl Probe {
        pub async fn new() -> Probe {
            Probe
        }

        pub async fn is_locked(&mut self) -> Option<bool> {
            // SAFETY: the dictionary is returned retained ("Copy" rule) and
            // released when `dict` is dropped.
            unsafe {
                let raw = CGSessionCopyCurrentDictionary();
                if raw.is_null() {
                    return None;
                }
                let dict: CFDictionary<CFString, CFType> =
                    CFDictionary::wrap_under_create_rule(raw);
                let key = CFString::from_static_string("CGSSessionScreenIsLocked");
                Some(
                    dict.find(&key)
                        .and_then(|v| v.downcast::<CFBoolean>())
                        .is_some_and(bool::from),
                )
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod imp {
    pub struct Probe;

    impl Probe {
        pub async fn new() -> Probe {
            Probe
        }

        pub async fn is_locked(&mut self) -> Option<bool> {
            None
        }
    }
}

pub use imp::Probe;
