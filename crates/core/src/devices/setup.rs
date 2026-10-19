//! Writes a device's settings into `config.yaml` (used by `aw-manual devices
//! setup`). Comments and unrelated settings stay untouched; only keys that
//! differ from the defaults (or from what was configured before) are written.

use super::{driver, DeviceConfig, DEFAULT_LONG_PRESS};
use crate::config::parse_config;
use crate::duration::format_duration;
use crate::yaml_edit::{set_value, Value};

/// Returns `src` with `new` applied to `devices.<driver>`. The result is
/// validated; nothing is returned if it would be an invalid configuration.
pub fn apply_to_config(src: &str, new: &DeviceConfig) -> Result<String, String> {
    let drv = driver(&new.driver).ok_or_else(|| format!("unknown driver \"{}\"", new.driver))?;
    let validation = |s: &str| {
        parse_config(s).map_err(|errs| {
            errs.into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ")
        })
    };
    let old = validation(src)?
        .devices
        .into_iter()
        .find(|d| d.driver == new.driver);
    let defaults = DeviceConfig::new(drv);
    let mut out = src.to_string();
    let mut set = |key: &[&str], value: Value| -> Result<(), String> {
        let mut path = vec!["devices", drv.id()];
        path.extend_from_slice(key);
        out = set_value(&out, &path, &value).map_err(|e| e.to_string())?;
        Ok(())
    };

    set(&["enabled"], Value::Bool(new.enabled))?;

    let text = |p: &std::path::PathBuf| p.to_string_lossy().into_owned();
    let fields = [
        (
            "name",
            new.selector.name.clone(),
            old.as_ref().and_then(|o| o.selector.name.clone()),
        ),
        (
            "usb_id",
            new.selector.usb_id.clone(),
            old.as_ref().and_then(|o| o.selector.usb_id.clone()),
        ),
        (
            "path",
            new.selector.path.as_ref().map(text),
            old.as_ref()
                .and_then(|o| o.selector.path.as_ref().map(text)),
        ),
    ];
    for (key, new_value, old_value) in fields {
        match (new_value, old_value) {
            (Some(v), _) => set(&["device", key], Value::Str(v))?,
            // A leftover field would still have to match.
            (None, Some(_)) => set(&["device", key], Value::Bool(false))?,
            (None, None) => {}
        }
    }

    if new.long_press != DEFAULT_LONG_PRESS
        || old
            .as_ref()
            .is_some_and(|o| o.long_press != DEFAULT_LONG_PRESS)
    {
        set(&["long_press"], Value::Str(format_duration(new.long_press)))?;
    }

    for input in drv.inputs() {
        let name = input.name;
        let wanted = new.action(name);
        let is_default = wanted == defaults.action(name);
        let was_custom = old
            .as_ref()
            .is_some_and(|o| o.action(name) != defaults.action(name));
        if !is_default || was_custom {
            let value = wanted.map_or_else(|| "none".to_string(), ToString::to_string);
            set(&["bindings", name], Value::Str(value))?;
        }
    }

    validation(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_CONFIG_YAML;
    use crate::devices::{timebuzzer::TimeBuzzer, DeviceAction};

    fn device() -> DeviceConfig {
        let mut d = DeviceConfig::new(&TimeBuzzer);
        d.enabled = true;
        d.selector.usb_id = Some("abcd:0012".into());
        d
    }

    fn parsed(src: &str) -> DeviceConfig {
        parse_config(src).unwrap().devices.remove(0)
    }

    #[test]
    fn adds_the_section_and_keeps_comments() {
        let out = apply_to_config(DEFAULT_CONFIG_YAML, &device()).unwrap();
        assert_eq!(parsed(&out), device());
        assert!(out.contains("# aw-manual base configuration"));
        assert!(out.contains("# devices:"));
        // Defaults are not written out.
        let active: String = out
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect();
        assert!(!active.contains("long_press"));
        assert!(!active.contains("bindings"));
    }

    #[test]
    fn writes_only_changed_bindings_and_long_press() {
        let mut d = device();
        d.long_press = std::time::Duration::from_secs(1);
        d.bindings
            .retain(|(n, _)| !["press", "hover_start", "long_press"].contains(&n.as_str()));
        d.bindings.push((
            "long_press".into(),
            DeviceAction::Favorite("acme/dev".into()),
        ));
        let out = apply_to_config(DEFAULT_CONFIG_YAML, &d).unwrap();
        let got = parsed(&out);
        assert_eq!(got, d);
    }

    #[test]
    fn replaces_an_existing_selection_completely() {
        let first = {
            let mut d = device();
            d.selector.usb_id = None;
            d.selector.name = Some("old".into());
            d.long_press = std::time::Duration::from_secs(2);
            apply_to_config(DEFAULT_CONFIG_YAML, &d).unwrap()
        };
        let out = apply_to_config(&first, &device()).unwrap();
        assert_eq!(parsed(&out), device());
    }

    #[test]
    fn invalid_result_is_an_error() {
        let mut d = device();
        d.selector.usb_id = Some("nope".into());
        assert!(apply_to_config(DEFAULT_CONFIG_YAML, &d).is_err());
    }
}
