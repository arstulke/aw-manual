//! Backend of the settings window: reads and writes the same YAML files as a
//! user would, keeping comments and order (see `aw_manual_core::yaml_edit`).

use std::path::{Path, PathBuf};

use aw_manual_core::config::{
    parse_config, CONFIG_FILE, DEFAULT_CONFIG_YAML, DEFAULT_FAVORITES_DIR,
};
use aw_manual_core::favorites::{is_favorites_file, parse_favorites_file};
use aw_manual_core::yaml_edit::{self, NewFavorite, Value as YamlValue};
use serde::Deserialize;
use serde_json::Value;

use aw_manual_core::service::{Service, Ui};

#[derive(Debug, Deserialize)]
pub struct Change {
    pub path: Vec<String>,
    pub value: Value,
}

/// Writes a file atomically (temporary file + rename), so the file watcher
/// never sees a half-written file.
pub fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

fn to_yaml_value(v: &Value) -> Result<YamlValue, String> {
    Ok(match v {
        Value::String(s) => YamlValue::Str(s.clone()),
        Value::Bool(b) => YamlValue::Bool(*b),
        Value::Number(n) => YamlValue::Num(n.as_f64().ok_or("invalid number")?),
        Value::Array(items) => YamlValue::List(
            items
                .iter()
                .map(|i| {
                    i.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "lists may only contain text".to_string())
                })
                .collect::<Result<_, _>>()?,
        ),
        other => return Err(format!("unsupported value {other}")),
    })
}

/// Applies changes to `config.yaml`. Nothing is written if the result is invalid.
pub fn save_config(config_dir: &Path, changes: &[Change]) -> Result<(), Vec<String>> {
    let path = config_dir.join(CONFIG_FILE);
    let mut src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_CONFIG_YAML.to_string(),
        Err(e) => return Err(vec![format!("cannot read {}: {e}", path.display())]),
    };
    for c in changes {
        if c.path.is_empty() {
            return Err(vec!["empty setting path".into()]);
        }
        let value = to_yaml_value(&c.value).map_err(|e| vec![e])?;
        let p: Vec<&str> = c.path.iter().map(String::as_str).collect();
        src = yaml_edit::set_value(&src, &p, &value)
            .map_err(|e| vec![format!("{}: {e}", c.path.join("."))])?;
    }
    parse_config(&src).map_err(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())?;
    write_atomic(&path, &src).map_err(|e| vec![format!("cannot write {}: {e}", path.display())])
}

/// ID segments of a favorite inside the file that defines it (without the file's `parent`).
fn segments_in_file(file: &Path, id: &str) -> Result<Vec<String>, String> {
    let src = std::fs::read_to_string(file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let parsed = parse_favorites_file(&src)
        .map_err(|_| format!("{} contains errors; please fix it first", file.display()))?;
    let rel = match &parsed.parent {
        Some(p) => id
            .strip_prefix(&format!("{p}/"))
            .ok_or_else(|| format!("\"{id}\" is not defined in {}", file.display()))?,
        None => id,
    };
    Ok(rel.split('/').map(str::to_string).collect())
}

fn check_writable(file: &Path) -> Result<(), String> {
    if let Ok(src) = std::fs::read_to_string(file) {
        if let Ok(p) = parse_favorites_file(&src) {
            if p.generated {
                return Err(format!(
                    "{} is generated and read-only in the settings",
                    file.display()
                ));
            }
        }
    }
    Ok(())
}

fn first_favorites_dir<U: Ui>(svc: &Service<U>) -> PathBuf {
    svc.config()
        .favorite_dirs(svc.config_dir())
        .into_iter()
        .next()
        .unwrap_or_else(|| svc.config_dir().join(DEFAULT_FAVORITES_DIR))
}

/// Adds a favorite below `parent` (a folder ID) or at the top level.
/// Top-level favorites go to `file` (one of [`writable_files`]) or to
/// `favorites.default_file` in the first favorites folder; favorites in a folder go to the file defining that folder.
pub fn add_favorite<U: Ui>(
    svc: &Service<U>,
    parent: Option<&str>,
    file: Option<&Path>,
    fav: &NewFavorite,
) -> Result<String, String> {
    let segment = fav.validate().map_err(|e| e.0)?;
    let (file, in_file, full_id) = {
        let loaded = svc.loaded();
        let tree = &loaded.favorites.tree;
        match parent.filter(|p| !p.is_empty()) {
            None => match file {
                None => (
                    first_favorites_dir(svc).join(&svc.config().favorites.default_file),
                    Vec::new(),
                    segment.clone(),
                ),
                Some(f) => {
                    if !writable_files(svc).iter().any(|w| w == f) {
                        return Err(format!("{} is not an editable favorites file", f.display()));
                    }
                    // A file with `parent` attaches its top level below that folder.
                    let src = std::fs::read_to_string(f)
                        .map_err(|e| format!("cannot read {}: {e}", f.display()))?;
                    let parsed = parse_favorites_file(&src).map_err(|_| {
                        format!("{} contains errors; please fix it first", f.display())
                    })?;
                    let full = match &parsed.parent {
                        Some(p) => format!("{p}/{segment}"),
                        None => segment.clone(),
                    };
                    (f.to_path_buf(), Vec::new(), full)
                }
            },
            Some(pid) => {
                let folder = tree
                    .find(pid)
                    .ok_or_else(|| format!("folder \"{pid}\" not found"))?;
                if folder.is_leaf() {
                    return Err(format!(
                        "\"{pid}\" is a leaf; favorites can only be added to folders"
                    ));
                }
                if folder.generated {
                    return Err(format!(
                        "\"{pid}\" comes from a generated file and cannot be edited here"
                    ));
                }
                (
                    folder.file.clone(),
                    segments_in_file(&folder.file, pid)?,
                    format!("{pid}/{segment}"),
                )
            }
        }
    };
    if svc.loaded().favorites.tree.find(&full_id).is_some() {
        return Err(format!(
            "a favorite with the ID \"{full_id}\" already exists"
        ));
    }
    check_writable(&file)?;
    let src = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    let segs: Vec<&str> = in_file.iter().map(String::as_str).collect();
    let out = yaml_edit::add_favorite(&src, &segs, fav)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    parse_favorites_file(&out)
        .map_err(|e| format!("the change would make {} invalid: {}", file.display(), e[0]))?;
    write_atomic(&file, &out).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    Ok(full_id)
}

pub fn remove_favorite<U: Ui>(svc: &Service<U>, id: &str) -> Result<(), String> {
    let file = {
        let loaded = svc.loaded();
        let f = loaded
            .favorites
            .tree
            .find(id)
            .ok_or_else(|| format!("favorite \"{id}\" not found"))?;
        if f.generated {
            return Err(format!(
                "\"{id}\" comes from a generated file and cannot be edited here"
            ));
        }
        f.file.clone()
    };
    check_writable(&file)?;
    let segs_owned = segments_in_file(&file, id)?;
    let segs: Vec<&str> = segs_owned.iter().map(String::as_str).collect();
    let src = std::fs::read_to_string(&file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let out =
        yaml_edit::remove_favorite(&src, &segs).map_err(|e| format!("{}: {e}", file.display()))?;
    parse_favorites_file(&out)
        .map_err(|e| format!("the change would make {} invalid: {}", file.display(), e[0]))?;
    write_atomic(&file, &out).map_err(|e| format!("cannot write {}: {e}", file.display()))
}

/// Files the settings UI can write to (non-generated favorites files).
pub fn writable_files<U: Ui>(svc: &Service<U>) -> Vec<PathBuf> {
    svc.favorite_files()
        .into_iter()
        .filter(|f| {
            !f.generated
                && f.path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_favorites_file)
        })
        .map(|f| f.path)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_config_keeps_comments_and_validates() {
        let dir = std::env::temp_dir().join(format!("aw-manual-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Missing file: starts from the commented default.
        save_config(
            &dir,
            &[Change {
                path: vec!["tray".into(), "display".into()],
                value: Value::String("elapsed".into()),
            }],
        )
        .unwrap();
        let src = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(src.contains("display: elapsed"));
        assert!(src.contains("# a remote aw-server is possible as well"));
        // Invalid values are rejected and nothing is written.
        let errs = save_config(
            &dir,
            &[Change {
                path: vec!["popover".into(), "scroll".into(), "dwell_time".into()],
                value: Value::String("0s".into()),
            }],
        )
        .unwrap_err();
        assert!(errs[0].contains("dwell_time"), "{errs:?}");
        assert_eq!(std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap(), src);
        // Lists and booleans.
        save_config(
            &dir,
            &[
                Change {
                    path: vec!["notifications".into(), "work_hours".into(), "days".into()],
                    value: serde_json::json!(["mon", "wed"]),
                },
                Change {
                    path: vec!["notifications".into(), "idle_reminder".into()],
                    value: Value::Bool(false),
                },
                Change {
                    path: vec!["favorites".into(), "dirs".into()],
                    value: serde_json::json!(["favorites.d", "~/Sync/favorites"]),
                },
                Change {
                    path: vec!["favorites".into(), "default_file".into()],
                    value: Value::String("05-mine.yaml".into()),
                },
            ],
        )
        .unwrap();
        let c = parse_config(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap()).unwrap();
        assert_eq!(c.notifications.work_hours.days.len(), 2);
        assert_eq!(c.notifications.idle_reminder, None);
        assert_eq!(c.favorites.dirs, vec!["favorites.d", "~/Sync/favorites"]);
        assert_eq!(c.favorites.default_file, "05-mine.yaml");
        assert!(!dir.join(format!(".{CONFIG_FILE}.tmp")).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn favorites_are_added_to_the_right_files() {
        use aw_manual_core::service::Headless;
        let dir =
            std::env::temp_dir().join(format!("aw-manual-settings-fav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let fav_dir = dir.join("cfg/favorites.d");
        std::fs::create_dir_all(&fav_dir).unwrap();
        std::fs::write(
            fav_dir.join("10-clients.yaml"),
            "# clients\nfavorites:\n  ACME:\n    Website: [Development]\n    Ops:\n      Deploy:\n",
        )
        .unwrap();
        std::fs::write(
            fav_dir.join("20-extra.yaml"),
            "parent: acme\nfavorites:\n  Support:\n",
        )
        .unwrap();
        std::fs::write(
            fav_dir.join("50-gen.yaml"),
            "generated: true\nfavorites:\n  - id: g\n    title: G\n",
        )
        .unwrap();
        let shutdown = std::sync::Arc::new(tokio::sync::Notify::new());
        let svc = Service::start(
            dir.join("cfg"),
            &dir.join("db.sqlite"),
            Headless { shutdown },
        )
        .unwrap();
        let fav = |t: &str| NewFavorite {
            title: t.into(),
            id: None,
            kind: None,
            color: None,
            icon: None,
            ask_note: false,
        };

        // Top level: 00-ui.yaml (created).
        assert_eq!(
            add_favorite(&svc, None, None, &fav("Lunch")).unwrap(),
            "lunch"
        );
        assert!(std::fs::read_to_string(
            fav_dir.join(aw_manual_core::config::DEFAULT_FAVORITES_FILE)
        )
        .unwrap()
        .contains("Lunch:"));
        // Into a folder: the file that defines the folder.
        svc.reload().await;
        assert_eq!(
            add_favorite(&svc, Some("acme/ops"), None, &fav("Rollback")).unwrap(),
            "acme/ops/rollback"
        );
        let clients = std::fs::read_to_string(fav_dir.join("10-clients.yaml")).unwrap();
        assert!(
            clients.starts_with("# clients\n") && clients.contains("      Rollback:\n"),
            "{clients}"
        );
        // Top level of a picked file with `parent`: attached below that folder.
        svc.reload().await;
        let extra = fav_dir.join("20-extra.yaml");
        assert_eq!(
            add_favorite(&svc, None, Some(&extra), &fav("Training")).unwrap(),
            "acme/training"
        );
        svc.reload().await;
        assert!(svc.loaded().favorites.tree.find("acme/training").is_some());
        // Folders defined in a file with `parent`.
        assert!(svc.loaded().favorites.tree.find("acme/support").is_some());
        // Errors: duplicates, leaves, generated files, foreign files.
        assert!(add_favorite(&svc, None, None, &fav("Lunch"))
            .unwrap_err()
            .contains("already exists"));
        assert!(
            add_favorite(&svc, Some("acme/website/development"), None, &fav("X"))
                .unwrap_err()
                .contains("leaf")
        );
        assert!(add_favorite(&svc, None, Some(&fav_dir.join("50-gen.yaml")), &fav("X")).is_err());
        assert!(add_favorite(&svc, None, Some(&dir.join("elsewhere.yaml")), &fav("X")).is_err());
        assert!(remove_favorite(&svc, "g")
            .unwrap_err()
            .contains("generated"));
        // Removal from the defining file, also below a `parent`.
        remove_favorite(&svc, "acme/training").unwrap();
        remove_favorite(&svc, "acme/ops/deploy").unwrap();
        svc.reload().await;
        let t = &svc.loaded().favorites.tree;
        assert!(t.find("acme/training").is_none());
        assert!(t.find("acme/ops/deploy").is_none());
        assert!(t.find("acme/ops/rollback").is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
