//! Activities, runs and the data written to ActivityWatch events.

use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};

use crate::favorites::{Favorite, Kind};
use crate::format::PATH_SEPARATOR;

/// Where a command came from (`data.source` in events).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Popover,
    PopoverScroll,
    Tray,
    Cli,
    Openaction,
    Hotkey,
    Device,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Popover => "popover",
            Source::PopoverScroll => "popover-scroll",
            Source::Tray => "tray",
            Source::Cli => "cli",
            Source::Openaction => "openaction",
            Source::Hotkey => "hotkey",
            Source::Device => "device",
        }
    }
}

/// How the activity was defined (`data.origin` in events).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// A favorite from the configuration.
    Config,
    /// Defined on an OpenAction key (Custom Activity or orphaned favorite).
    Deck,
    /// Defined freely via the CLI.
    Adhoc,
}

/// `data.entry` in events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Entry {
    Timer,
    Retro,
}

/// A startable activity: a favorite leaf or an ad-hoc definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub id: String,
    pub title: String,
    pub path: Vec<String>,
    pub kind: Kind,
    pub origin: Origin,
    /// Presentation only (tray tint); never written into events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl Activity {
    /// Builds an activity from a favorite leaf.
    pub fn from_favorite(f: &Favorite) -> Activity {
        Activity {
            id: f.id.clone(),
            title: f.title.clone(),
            path: f.path.clone(),
            kind: f.leaf_kind(),
            origin: Origin::Config,
            color: f.color.as_ref().map(|c| c.value.clone()),
            icon: f.icon.as_ref().map(|c| c.value.clone()),
        }
    }

    /// True if both describe the same activity (same ID and origin).
    pub fn same_as(&self, other: &Activity) -> bool {
        self.id == other.id && self.origin == other.origin
    }
}

/// The `data` object of an ActivityWatch event.
///
/// `title` is the full path ("ACME › Website › Development"), so viewers that
/// only show the title (e.g. the ActivityWatch timeline) show where the
/// activity belongs; `path` keeps the individual titles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventData {
    pub id: String,
    pub title: String,
    pub path: Vec<String>,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub entry: Entry,
    pub origin: Origin,
}

impl EventData {
    pub fn new(
        activity: &Activity,
        note: Option<&str>,
        source: Source,
        device: Option<&str>,
        entry: Entry,
    ) -> Self {
        EventData {
            id: activity.id.clone(),
            title: event_title(activity),
            path: activity.path.clone(),
            kind: activity.kind,
            note: note.map(str::to_string),
            source,
            device: device.map(str::to_string),
            entry,
            origin: activity.origin,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("event data serializes")
    }
}

/// The title written into events: the full path, or the title if the path is empty.
pub fn event_title(activity: &Activity) -> String {
    if activity.path.is_empty() {
        activity.title.clone()
    } else {
        activity.path.join(PATH_SEPARATOR)
    }
}

/// The currently running activity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub activity: Activity,
    pub note: Option<String>,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Start of the current event (moves when a retroactive entry overlaps the run).
    pub start: DateTime<Utc>,
    /// Time of the last heartbeat that was queued.
    pub last_heartbeat: DateTime<Utc>,
    /// Whether the long-running notification was shown for this run.
    #[serde(default)]
    pub long_running_notified: bool,
}

impl Run {
    pub fn event_data(&self) -> EventData {
        EventData::new(
            &self.activity,
            self.note.as_deref(),
            self.source,
            self.device.as_deref(),
            Entry::Timer,
        )
    }
}

/// An activity that was stopped (for "resume").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stopped {
    pub activity: Activity,
    pub note: Option<String>,
    pub ended: DateTime<Utc>,
}

/// Normalizes a timestamp to millisecond precision, so it round-trips
/// through aw-server unchanged.
pub fn trunc_ms(t: DateTime<Utc>) -> DateTime<Utc> {
    t.trunc_subsecs(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(path: &[&str]) -> Activity {
        Activity {
            id: "acme/website/dev".into(),
            title: "Development".into(),
            path: path.iter().map(|s| s.to_string()).collect(),
            kind: Kind::Activity,
            origin: Origin::Config,
            color: None,
            icon: None,
        }
    }

    #[test]
    fn event_title_is_the_full_path() {
        let a = activity(&["ACME", "Website", "Development"]);
        let d = EventData::new(&a, None, Source::Cli, None, Entry::Timer);
        assert_eq!(d.title, "ACME › Website › Development");
        assert_eq!(d.path, vec!["ACME", "Website", "Development"]);
        assert_eq!(event_title(&activity(&[])), "Development");
    }
}
