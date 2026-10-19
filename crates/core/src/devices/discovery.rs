//! Finds the connected devices of a transport, so users can look up the
//! values for `devices.<driver>.device` instead of guessing them.
//!
//! MIDI (Linux): `/proc/asound/cards` lists the ALSA cards, `/proc/asound/card<N>/usbid`
//! the USB ID of USB cards, `/dev/snd/midiC<N>D<M>` the raw MIDI nodes.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{DeviceSelector, Transport};

/// A device that is connected right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    pub transport: Transport,
    /// ALSA card ID, e.g. the name shown in `/proc/asound/cards`.
    pub name: String,
    /// Longer description (card driver and name).
    pub description: String,
    /// USB `vendor:product` (lowercase hex) for USB devices.
    pub usb_id: Option<String>,
    pub card: u32,
    /// Raw MIDI nodes of the card.
    pub nodes: Vec<PathBuf>,
    /// Whether the current user may read the first node.
    pub readable: bool,
}

/// Where the system files are. Replaceable for tests.
#[derive(Debug, Clone)]
pub struct Roots {
    pub proc_asound: PathBuf,
    pub dev_snd: PathBuf,
}

impl Default for Roots {
    fn default() -> Self {
        Roots {
            proc_asound: "/proc/asound".into(),
            dev_snd: "/dev/snd".into(),
        }
    }
}

/// Lists the connected MIDI devices (cards that have a raw MIDI node).
pub fn discover_midi(roots: &Roots) -> Result<Vec<DeviceInfo>, String> {
    if !cfg!(target_os = "linux") && roots.proc_asound == Path::new("/proc/asound") {
        return Err("device discovery is only available on Linux".into());
    }
    let cards = match std::fs::read_to_string(roots.proc_asound.join("cards")) {
        Ok(c) => c,
        // No ALSA at all (no sound drivers loaded): nothing connected.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(format!(
                "cannot read {}/cards: {e}",
                roots.proc_asound.display()
            ))
        }
    };
    let nodes: Vec<String> = std::fs::read_dir(&roots.dev_snd)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for (card, name, description) in parse_cards(&cards) {
        let prefix = format!("midiC{card}D");
        let mut card_nodes: Vec<PathBuf> = nodes
            .iter()
            .filter(|n| {
                n.strip_prefix(&prefix).is_some_and(|rest| {
                    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
                })
            })
            .map(|n| roots.dev_snd.join(n))
            .collect();
        if card_nodes.is_empty() {
            continue;
        }
        card_nodes.sort();
        let usb_id = std::fs::read_to_string(roots.proc_asound.join(format!("card{card}/usbid")))
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| super::validate_usb_id(s).is_ok());
        let readable = is_readable(&card_nodes[0]);
        out.push(DeviceInfo {
            transport: Transport::Midi,
            name,
            description,
            usb_id,
            card,
            nodes: card_nodes,
            readable,
        });
    }
    Ok(out)
}

/// Parses `/proc/asound/cards`:
/// ```text
///  5 [timeBuzzer      ]: USB-Audio - timeBuzzer
///                       Vendor timeBuzzer at usb-0000:00:14.0-1, full speed
/// ```
/// Returns (card number, card ID, description).
pub fn parse_cards(text: &str) -> Vec<(u32, String, String)> {
    text.lines()
        .filter_map(|line| {
            let (num, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let card: u32 = num.parse().ok()?;
            let rest = rest.trim_start().strip_prefix('[')?;
            let (id, rest) = rest.split_once(']')?;
            let description = rest.trim_start_matches(':').trim();
            Some((card, id.trim().to_string(), description.to_string()))
        })
        .collect()
}

#[cfg(unix)]
fn is_readable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .is_ok_and(|p| unsafe { libc::access(p.as_ptr(), libc::R_OK) } == 0)
}

#[cfg(not(unix))]
fn is_readable(path: &Path) -> bool {
    path.exists()
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    NotConnected,
    /// The selector matched nothing, but other devices exist (their names).
    NoMatch(Vec<String>),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::NotConnected => f.write_str("no MIDI device connected"),
            ResolveError::NoMatch(found) => write!(
                f,
                "no connected device matches the selection (connected: {})",
                found.join(", ")
            ),
        }
    }
}

/// Picks the device node for a selector. With `path`, that node is used
/// as is (the other fields still have to match a device that owns it, if
/// given).
pub fn resolve(selector: &DeviceSelector, devices: &[DeviceInfo]) -> Result<PathBuf, ResolveError> {
    if let (Some(path), None, None) = (&selector.path, &selector.name, &selector.usb_id) {
        return Ok(path.clone());
    }
    let matches = |d: &&DeviceInfo| {
        let name_ok = selector.name.as_ref().is_none_or(|n| {
            let n = n.to_lowercase();
            d.name.to_lowercase().contains(&n) || d.description.to_lowercase().contains(&n)
        });
        let usb_ok = selector.usb_id.as_ref().is_none_or(|id| {
            d.usb_id
                .as_deref()
                .is_some_and(|u| u.eq_ignore_ascii_case(id))
        });
        let path_ok = selector.path.as_ref().is_none_or(|p| d.nodes.contains(p));
        name_ok && usb_ok && path_ok
    };
    if devices.is_empty() {
        return Err(ResolveError::NotConnected);
    }
    devices
        .iter()
        .filter(matches)
        .min_by_key(|d| d.card)
        .and_then(|d| d.nodes.first().cloned())
        .ok_or_else(|| ResolveError::NoMatch(devices.iter().map(|d| d.name.clone()).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const CARDS: &str = " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n                      HDA Intel PCH at 0xf7210000 irq 130\n 5 [timeBuzzer     ]: USB-Audio - timeBuzzer\n                      Vendor timeBuzzer at usb-0000:00:14.0-1, full speed\n";

    fn setup(name: &str) -> Roots {
        let base =
            std::env::temp_dir().join(format!("aw-manual-discovery-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let proc = base.join("proc");
        let dev = base.join("dev");
        fs::create_dir_all(proc.join("card5")).unwrap();
        fs::create_dir_all(&dev).unwrap();
        fs::write(proc.join("cards"), CARDS).unwrap();
        fs::write(proc.join("card5/usbid"), "ABCD:0012\n").unwrap();
        for n in ["midiC5D0", "midiC5D1", "pcmC5D0p", "midiC0D0x", "controlC5"] {
            fs::write(dev.join(n), "").unwrap();
        }
        Roots {
            proc_asound: proc,
            dev_snd: dev,
        }
    }

    #[test]
    fn parses_cards() {
        assert_eq!(
            parse_cards(CARDS),
            vec![
                (0, "PCH".into(), "HDA-Intel - HDA Intel PCH".into()),
                (5, "timeBuzzer".into(), "USB-Audio - timeBuzzer".into()),
            ]
        );
    }

    #[test]
    fn discovers_midi_cards_only() {
        let roots = setup("list");
        let devices = discover_midi(&roots).unwrap();
        assert_eq!(
            devices,
            vec![DeviceInfo {
                transport: Transport::Midi,
                name: "timeBuzzer".into(),
                description: "USB-Audio - timeBuzzer".into(),
                usb_id: Some("abcd:0012".into()),
                card: 5,
                nodes: vec![
                    roots.dev_snd.join("midiC5D0"),
                    roots.dev_snd.join("midiC5D1")
                ],
                readable: true,
            }]
        );
    }

    #[test]
    fn no_alsa_means_no_devices() {
        let roots = Roots {
            proc_asound: "/nonexistent/aw-manual".into(),
            dev_snd: "/nonexistent/aw-manual".into(),
        };
        assert_eq!(discover_midi(&roots).unwrap(), vec![]);
    }

    #[test]
    fn resolves_by_name_usb_id_and_path() {
        let roots = setup("resolve");
        let devices = discover_midi(&roots).unwrap();
        let node = roots.dev_snd.join("midiC5D0");
        let sel = |name: Option<&str>, usb: Option<&str>, path: Option<&Path>| DeviceSelector {
            name: name.map(Into::into),
            usb_id: usb.map(Into::into),
            path: path.map(Into::into),
        };
        assert_eq!(
            resolve(&sel(Some("timebuzzer"), None, None), &devices),
            Ok(node.clone())
        );
        assert_eq!(
            resolve(&sel(None, Some("ABCD:0012"), None), &devices),
            Ok(node.clone())
        );
        assert_eq!(
            resolve(&sel(Some("buzz"), Some("abcd:0012"), None), &devices),
            Ok(node.clone())
        );
        let fixed = Path::new("/dev/snd/midiC9D0");
        assert_eq!(
            resolve(&sel(None, None, Some(fixed)), &[]),
            Ok(fixed.into())
        );
        assert_eq!(
            resolve(&sel(Some("other"), None, None), &devices),
            Err(ResolveError::NoMatch(vec!["timeBuzzer".into()]))
        );
        assert_eq!(
            resolve(&sel(None, Some("1111:2222"), None), &devices),
            Err(ResolveError::NoMatch(vec!["timeBuzzer".into()]))
        );
        assert_eq!(
            resolve(&sel(Some("x"), None, None), &[]),
            Err(ResolveError::NotConnected)
        );
    }
}
