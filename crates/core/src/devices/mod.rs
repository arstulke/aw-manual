//! Hardware input devices (timeBuzzer, later e.g. Stream Deck).
//!
//! A [`Driver`] knows how to decode what a kind of device sends into
//! [`Signal`]s. It never knows *which* hardware to open: users select it in
//! `devices.<driver>.device` with values reported by `aw-manual devices list`
//! ([`discovery`]), so no hardware IDs or `/dev` paths are compiled in.
//!
//! The pipeline is: device node → bytes ([`runner`]) → driver [`Decoder`] →
//! [`Signal`]s → [`Gestures`] (press vs. long press) → named *input* →
//! configured [`DeviceAction`].
//!
//! To support another device, implement [`Driver`] in its own module and add
//! it to [`drivers`]; configuration, discovery, CLI and daemon need no change.

pub mod discovery;
pub mod gestures;
pub mod midi;
pub mod runner;
pub mod setup;
pub mod timebuzzer;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Serialize, Serializer};

use crate::ipc::NavCommand;

pub use gestures::Gestures;

/// How a device talks to the computer. Discovery and the byte reader are
/// specific to the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Transport {
    /// ALSA raw MIDI node (`/dev/snd/midiC<card>D<device>`).
    Midi,
}

/// What a decoder reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// A button went down or up. [`Gestures`] turns it into `<name>` or
    /// `long_<name>` once it is released.
    Button { name: &'static str, down: bool },
    /// A one-shot input, such as one step of a knob.
    Trigger(&'static str),
}

/// Turns bytes of one device into [`Signal`]s. One instance per connection.
pub trait Decoder: Send {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Signal>;
}

/// Description of one bindable input of a driver.
#[derive(Debug, Clone, Copy)]
pub struct InputInfo {
    pub name: &'static str,
    pub description: &'static str,
}

/// A kind of device.
pub trait Driver: Sync {
    /// Key below `devices:` in `config.yaml`.
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn transport(&self) -> Transport;
    /// All names that can be bound in `devices.<id>.bindings`.
    fn inputs(&self) -> &'static [InputInfo];
    /// Bindings used for inputs the configuration doesn't mention.
    fn default_bindings(&self) -> Vec<(&'static str, DeviceAction)>;
    fn decoder(&self) -> Box<dyn Decoder>;
}

/// All supported drivers.
pub fn drivers() -> &'static [&'static dyn Driver] {
    &[&timebuzzer::TimeBuzzer]
}

pub fn driver(id: &str) -> Option<&'static dyn Driver> {
    drivers().iter().copied().find(|d| d.id() == id)
}

/// What an input does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceAction {
    /// Open the popover, or confirm when it is open (like the popover hotkey).
    Popover,
    /// Open the popover if it is closed; does nothing when it is open.
    OpenPopover,
    /// Close the popover if the `open` action opened it and nothing was
    /// confirmed in it since; otherwise does nothing.
    ClosePopover,
    Stop,
    Resume,
    /// Navigate the open popover (ignored while it is closed).
    Nav(NavCommand),
    /// Start a favorite by its ID.
    Favorite(String),
}

impl DeviceAction {
    /// Parses `popover`, `open`, `close`, `stop`, `resume`, `next`, `prev`, `confirm`, `back`,
    /// `cancel` or `favorite:<id>`. `Ok(None)` means "no action".
    pub fn parse(s: &str) -> Result<Option<DeviceAction>, String> {
        let s = s.trim();
        Ok(Some(match s {
            "" | "none" => return Ok(None),
            "popover" => DeviceAction::Popover,
            "open" => DeviceAction::OpenPopover,
            "close" => DeviceAction::ClosePopover,
            "stop" => DeviceAction::Stop,
            "resume" => DeviceAction::Resume,
            "next" => DeviceAction::Nav(NavCommand::Next),
            "prev" => DeviceAction::Nav(NavCommand::Prev),
            "confirm" => DeviceAction::Nav(NavCommand::Confirm),
            "back" => DeviceAction::Nav(NavCommand::Back),
            "cancel" => DeviceAction::Nav(NavCommand::Cancel),
            other => match other.strip_prefix("favorite:") {
                Some(id) if !id.trim().is_empty() => DeviceAction::Favorite(id.trim().into()),
                _ => {
                    return Err(format!(
                    "unknown action \"{other}\" (use popover, open, close, stop, resume, next, prev, confirm, back, cancel, favorite:<id> or none)"
                ))
                }
            },
        }))
    }
}

impl fmt::Display for DeviceAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceAction::Popover => f.write_str("popover"),
            DeviceAction::OpenPopover => f.write_str("open"),
            DeviceAction::ClosePopover => f.write_str("close"),
            DeviceAction::Stop => f.write_str("stop"),
            DeviceAction::Resume => f.write_str("resume"),
            DeviceAction::Nav(NavCommand::Next) => f.write_str("next"),
            DeviceAction::Nav(NavCommand::Prev) => f.write_str("prev"),
            DeviceAction::Nav(NavCommand::Confirm) => f.write_str("confirm"),
            DeviceAction::Nav(NavCommand::Back) => f.write_str("back"),
            DeviceAction::Nav(NavCommand::Cancel) => f.write_str("cancel"),
            DeviceAction::Favorite(id) => write!(f, "favorite:{id}"),
        }
    }
}

impl Serialize for DeviceAction {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

/// Which hardware to use. At least one field is set. If several are set, all
/// must match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeviceSelector {
    /// Card name (case-insensitive part of the card ID or description).
    pub name: Option<String>,
    /// USB `vendor:product` in hex, e.g. `abcd:1234`.
    pub usb_id: Option<String>,
    /// Fixed device node. Card numbers change between plug-ins, so prefer
    /// `name` or `usb_id`.
    pub path: Option<PathBuf>,
}

impl DeviceSelector {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.usb_id.is_none() && self.path.is_none()
    }
}

/// `usb_id` syntax: `hhhh:hhhh`.
pub fn validate_usb_id(s: &str) -> Result<(), String> {
    let ok = s.split_once(':').is_some_and(|(v, p)| {
        [v, p]
            .iter()
            .all(|x| x.len() == 4 && x.chars().all(|c| c.is_ascii_hexdigit()))
    });
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid USB ID \"{s}\" (expected vendor:product in hex, e.g. 1234:abcd)"
        ))
    }
}

/// Settings of one device (`devices.<driver>` in `config.yaml`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceConfig {
    pub driver: String,
    pub enabled: bool,
    #[serde(rename = "device")]
    pub selector: DeviceSelector,
    /// A button held at least this long counts as `long_<button>`.
    #[serde(serialize_with = "ser_duration")]
    pub long_press: Duration,
    /// Input name → action, complete (defaults merged with the configuration).
    pub bindings: Vec<(String, DeviceAction)>,
}

pub const DEFAULT_LONG_PRESS: Duration = Duration::from_millis(600);

fn ser_duration<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&crate::duration::format_duration(*d))
}

impl DeviceConfig {
    /// The defaults of a driver: disabled, nothing selected.
    pub fn new(driver: &dyn Driver) -> Self {
        DeviceConfig {
            driver: driver.id().into(),
            enabled: false,
            selector: DeviceSelector::default(),
            long_press: DEFAULT_LONG_PRESS,
            bindings: driver
                .default_bindings()
                .into_iter()
                .map(|(n, a)| (n.to_string(), a))
                .collect(),
        }
    }

    pub fn action(&self, input: &str) -> Option<&DeviceAction> {
        self.bindings
            .iter()
            .find(|(n, _)| n == input)
            .map(|(_, a)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_roundtrip() {
        for s in [
            "popover",
            "open",
            "close",
            "stop",
            "resume",
            "next",
            "prev",
            "confirm",
            "back",
            "cancel",
            "favorite:acme/dev",
        ] {
            let a = DeviceAction::parse(s).unwrap().unwrap();
            assert_eq!(a.to_string(), s);
        }
    }

    #[test]
    fn no_action_and_errors() {
        assert_eq!(DeviceAction::parse("none").unwrap(), None);
        assert_eq!(DeviceAction::parse("").unwrap(), None);
        assert!(DeviceAction::parse("favorite:").is_err());
        assert!(DeviceAction::parse("explode").is_err());
    }

    #[test]
    fn usb_ids() {
        assert!(validate_usb_id("1234:abCD").is_ok());
        for bad in ["1234", "1234:abc", "12345:abcd", "zzzz:abcd", ""] {
            assert!(validate_usb_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn registry() {
        assert!(driver("timebuzzer").is_some());
        assert!(driver("nope").is_none());
        let ids: Vec<_> = drivers().iter().map(|d| d.id()).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(ids.len(), unique.len());
    }
}
