//! Comment-preserving edits of YAML files (used by the settings UI).
//!
//! Instead of re-serializing a document (which would drop comments and
//! formatting), edits are applied to the source text using the exact spans
//! of the parsed nodes. Only block-style mappings can receive new keys; other
//! layouts produce an [`EditError`] asking the user to edit the file by hand.

use crate::favorites::{slugify, validate_color, validate_segment, Kind};
use crate::yaml::{self, Node, NodeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError(pub String);

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

fn err<T>(msg: impl Into<String>) -> Result<T, EditError> {
    Err(EditError(msg.into()))
}

/// A value to write.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Bool(bool),
    Num(f64),
    /// A flow list of strings, e.g. `[mon, tue]`.
    List(Vec<String>),
}

const RESERVED: &[&str] = &[
    "true", "false", "yes", "no", "on", "off", "null", "~", "y", "n",
];

/// Formats a string as a YAML scalar: plain when unambiguous, double-quoted otherwise.
pub fn scalar(s: &str) -> String {
    let plain_ok = !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_alphanumeric())
        && !s.ends_with(' ')
        && s.chars()
            .all(|c| c.is_alphanumeric() || " _.-+/".contains(c))
        && !RESERVED.contains(&s.to_ascii_lowercase().as_str())
        && s.parse::<f64>().is_err()
        && !s.contains(" #");
    if plain_ok {
        s.to_string()
    } else {
        serde_json::to_string(s).expect("strings serialize")
    }
}

fn format_value(v: &Value) -> String {
    match v {
        Value::Str(s) => scalar(s),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{n:.1}")
            } else {
                format!("{n}")
            }
        }
        Value::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(|i| scalar(i))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Maps character indices (as used by the parser) to byte offsets.
struct Text<'a> {
    src: &'a str,
    offsets: Vec<usize>,
}

impl<'a> Text<'a> {
    fn new(src: &'a str) -> Self {
        let mut offsets: Vec<usize> = src.char_indices().map(|(i, _)| i).collect();
        offsets.push(src.len());
        Text { src, offsets }
    }

    fn byte(&self, char_index: usize) -> usize {
        self.offsets[char_index.min(self.offsets.len() - 1)]
    }

    /// Byte offset of the start of the line containing `byte`.
    fn line_start(&self, byte: usize) -> usize {
        self.src[..byte].rfind('\n').map_or(0, |i| i + 1)
    }

    /// Byte offset just after the newline ending the line containing `byte`.
    fn line_end_incl(&self, byte: usize) -> usize {
        self.src[byte..]
            .find('\n')
            .map_or(self.src.len(), |i| byte + i + 1)
    }

    /// Byte offset of the end of the line (before the newline).
    fn line_end(&self, byte: usize) -> usize {
        self.src[byte..]
            .find('\n')
            .map_or(self.src.len(), |i| byte + i)
    }
}

fn splice(src: &str, start: usize, end: usize, with: &str) -> String {
    let mut s = String::with_capacity(src.len() + with.len());
    s.push_str(&src[..start]);
    s.push_str(with);
    s.push_str(&src[end..]);
    s
}

fn parse(src: &str) -> Result<Node, EditError> {
    yaml::parse(src).map_err(|e| {
        EditError(format!(
            "the file cannot be parsed ({e}); please fix it by hand first"
        ))
    })
}

fn block_map(node: &Node) -> Option<&[(Node, Node)]> {
    match &node.kind {
        NodeKind::Map {
            entries,
            flow: false,
        } => Some(entries),
        _ => None,
    }
}

/// Builds the text for a nested block `path: value` at `indent`.
fn nested_block(path: &[&str], value: &str, indent: usize) -> String {
    let mut out = String::new();
    for (i, key) in path.iter().enumerate() {
        out.push_str(&" ".repeat(indent + 2 * i));
        out.push_str(&scalar(key));
        out.push(':');
        if i == path.len() - 1 {
            out.push(' ');
            out.push_str(value);
        }
        out.push('\n');
    }
    out
}

/// Ensures the text ends with a newline before appending at the end.
fn with_trailing_newline(src: &str) -> String {
    if src.is_empty() || src.ends_with('\n') {
        src.to_string()
    } else {
        format!("{src}\n")
    }
}

/// Byte offset after the line on which a node ending at `end_char` ends.
fn after_line(t: &Text<'_>, end_char: usize) -> usize {
    let b = t.byte(end_char);
    if b > 0 && t.src.as_bytes()[b - 1] == b'\n' {
        b // the span already includes the newline (block scalars)
    } else {
        t.line_end_incl(b)
    }
}

/// Inserts complete lines at a line boundary `at` (adding a newline at EOF if needed).
fn insert_lines(src: &str, at: usize, block: &str) -> String {
    if at == src.len() && !src.is_empty() && !src.ends_with('\n') {
        splice(src, at, at, &format!("\n{block}"))
    } else {
        splice(src, at, at, block)
    }
}

/// Inserts `block` (complete lines) after the last entry of a block mapping.
fn insert_after_entries(src: &str, entries: &[(Node, Node)], block: &str) -> String {
    let t = Text::new(src);
    let end = entries
        .last()
        .map(|(k, v)| v.span.end.index.max(k.span.end.index))
        .unwrap_or(0);
    insert_lines(src, after_line(&t, end), block)
}

/// Sets `path` to `value`, creating missing mappings.
pub fn set_value(src: &str, path: &[&str], value: &Value) -> Result<String, EditError> {
    assert!(!path.is_empty());
    let root = parse(src)?;
    let formatted = format_value(value);
    if root.is_null() {
        return Ok(format!(
            "{}{}",
            with_trailing_newline(src),
            nested_block(path, &formatted, 0)
        ));
    }
    let Some(mut entries) = block_map(&root) else {
        return err("the file must be a block mapping to be edited by the settings UI");
    };
    let mut indent = entries.first().map_or(0, |(k, _)| k.span.start.col);
    for (depth, key) in path.iter().enumerate() {
        let last = depth == path.len() - 1;
        let found = entries.iter().find(|(k, _)| k.as_scalar() == Some(key));
        match found {
            None => {
                let block = nested_block(&path[depth..], &formatted, indent);
                return Ok(insert_after_entries(src, entries, &block));
            }
            Some((k, v)) => {
                let t = Text::new(src);
                if last {
                    let (start, end) = match &v.kind {
                        NodeKind::Null if v.span.start.index == v.span.end.index => {
                            // `key:` without value: insert after the colon.
                            let colon = t.byte(k.span.end.index);
                            let after = src[colon..]
                                .find(':')
                                .map(|i| colon + i + 1)
                                .unwrap_or(colon);
                            return Ok(splice(src, after, after, &format!(" {formatted}")));
                        }
                        NodeKind::Map { flow: false, .. } | NodeKind::Seq { flow: false, .. } => {
                            // Replace a block collection: from after the colon to its end.
                            let colon = t.byte(k.span.end.index);
                            let after = src[colon..]
                                .find(':')
                                .map(|i| colon + i + 1)
                                .unwrap_or(colon);
                            (after, t.line_end(t.byte(v.span.end.index.max(1) - 1)))
                        }
                        _ => (t.byte(v.span.start.index), t.byte(v.span.end.index)),
                    };
                    let replacement = if matches!(
                        v.kind,
                        NodeKind::Map { flow: false, .. } | NodeKind::Seq { flow: false, .. }
                    ) {
                        format!(" {formatted}")
                    } else {
                        formatted
                    };
                    return Ok(splice(src, start, end, &replacement));
                }
                match &v.kind {
                    NodeKind::Map {
                        flow: false,
                        entries: e,
                    } => {
                        indent = e.first().map_or(indent + 2, |(k, _)| k.span.start.col);
                        entries = e;
                    }
                    NodeKind::Null => {
                        // `section:` without content: add the nested block below it.
                        let block =
                            nested_block(&path[depth + 1..], &formatted, k.span.start.col + 2);
                        return Ok(insert_lines(src, after_line(&t, k.span.end.index), &block));
                    }
                    _ => {
                        return err(format!(
                            "\"{}\" is not written as an indented block; please edit it by hand",
                            path[..=depth].join(".")
                        ))
                    }
                }
            }
        }
    }
    unreachable!()
}

// --- Favorites -------------------------------------------------------------------

/// A favorite to add through the settings UI.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewFavorite {
    pub title: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

impl NewFavorite {
    /// Validates the fields and returns the ID segment.
    pub fn validate(&self) -> Result<String, EditError> {
        let title = self.title.trim();
        if title.is_empty() {
            return err("the title must not be empty");
        }
        if title.starts_with('_') {
            return err("the title must not start with \"_\"");
        }
        let id = match self.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => {
                validate_segment(id).map_err(EditError)?;
                id.to_string()
            }
            None => {
                let s = slugify(title);
                if s.is_empty() {
                    return err("an ID cannot be derived from this title; please enter an ID");
                }
                s
            }
        };
        if let Some(c) = self.color.as_deref().filter(|c| !c.is_empty()) {
            validate_color(c).map_err(EditError)?;
        }
        Ok(id)
    }

    fn compact_lines(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut out = format!("{pad}{}:\n", scalar(self.title.trim()));
        let inner = " ".repeat(indent + 2);
        if let Some(id) = self.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            out.push_str(&format!("{inner}_id: {}\n", scalar(id)));
        }
        if let Some(c) = self.color.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("{inner}_color: \"{c}\"\n"));
        }
        if let Some(i) = self
            .icon
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push_str(&format!("{inner}_icon: {}\n", scalar(i)));
        }
        if self.kind == Some(Kind::Status) {
            out.push_str(&format!("{inner}_kind: status\n"));
        }
        out
    }

    fn explicit_lines(&self, id: &str, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let inner = " ".repeat(indent + 2);
        let mut out = format!(
            "{pad}- id: {}\n{inner}title: {}\n",
            scalar(id),
            scalar(self.title.trim())
        );
        if let Some(c) = self.color.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("{inner}color: \"{c}\"\n"));
        }
        if let Some(i) = self
            .icon
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.push_str(&format!("{inner}icon: {}\n", scalar(i)));
        }
        if self.kind == Some(Kind::Status) {
            out.push_str(&format!("{inner}kind: status\n"));
        }
        out
    }
}

/// Where a node lives in a favorites file.
enum Located<'a> {
    /// A compact node: its key and value nodes, plus the mapping entries it belongs to.
    Compact { key: &'a Node, value: &'a Node },
    /// A title inside a list shorthand.
    ShorthandItem { item: &'a Node, list: &'a Node },
    /// An explicit node (a list item mapping) and its list.
    Explicit { item: &'a Node },
}

fn compact_segment(key: &Node, value: &Node) -> Option<String> {
    let title = key.as_scalar()?;
    if title.starts_with('_') {
        return None;
    }
    match value.get("_id").and_then(Node::as_scalar) {
        Some(id) => Some(id.trim().to_string()),
        None => Some(slugify(title)),
    }
}

/// Finds a node by its ID segments within one file.
fn locate<'a>(favorites: &'a Node, segments: &[&str]) -> Result<Located<'a>, EditError> {
    let mut level = favorites;
    let mut found: Option<Located<'a>> = None;
    for (i, seg) in segments.iter().enumerate() {
        let next = match &level.kind {
            NodeKind::Map { entries, .. } => entries
                .iter()
                .find(|(k, v)| compact_segment(k, v).as_deref() == Some(seg))
                .map(|(k, v)| Located::Compact { key: k, value: v }),
            NodeKind::Seq { items, .. } => items.iter().find_map(|item| match &item.kind {
                NodeKind::Scalar { value, .. } if slugify(value) == *seg => {
                    Some(Located::ShorthandItem { item, list: level })
                }
                NodeKind::Map { .. }
                    if item.get("id").and_then(Node::as_scalar).map(str::trim) == Some(seg) =>
                {
                    Some(Located::Explicit { item })
                }
                _ => None,
            }),
            _ => None,
        };
        let Some(next) = next else {
            return err(format!(
                "\"{}\" was not found in the file",
                segments[..=i].join("/")
            ));
        };
        if i < segments.len() - 1 {
            level = match &next {
                Located::Compact { value, .. } => value,
                Located::Explicit { item, .. } => match item.get("children") {
                    Some(c) => c,
                    None => {
                        return err(format!("\"{}\" has no children", segments[..=i].join("/")))
                    }
                },
                Located::ShorthandItem { .. } => {
                    return err(format!("\"{}\" has no children", segments[..=i].join("/")))
                }
            };
        }
        found = Some(next);
    }
    found.ok_or_else(|| EditError("empty path".into()))
}

/// Adds a favorite to a favorites file. `parent` are the ID segments of the
/// folder inside this file (empty: the file's top level).
pub fn add_favorite(src: &str, parent: &[&str], fav: &NewFavorite) -> Result<String, EditError> {
    let id = fav.validate()?;
    let root = parse(src)?;
    if root.is_null() {
        if !parent.is_empty() {
            return err("the folder was not found in the file");
        }
        return Ok(format!(
            "{}favorites:\n{}",
            with_trailing_newline(src),
            fav.compact_lines(2)
        ));
    }
    let Some(top) = block_map(&root) else {
        return err("the file must be a block mapping to be edited by the settings UI");
    };
    let Some((fkey, favorites)) = top.iter().find(|(k, _)| k.as_scalar() == Some("favorites"))
    else {
        return Ok(insert_after_entries(
            src,
            top,
            &format!("favorites:\n{}", fav.compact_lines(2)),
        ));
    };
    let t = Text::new(src);
    // The list (explicit) or mapping (compact) receiving the new node.
    let (container, container_indent): (&Node, usize) = if parent.is_empty() {
        (favorites, fkey.span.start.col + 2)
    } else {
        match locate(favorites, parent)? {
            Located::Compact { key, value } => (value, key.span.start.col + 2),
            Located::Explicit { item, .. } => match item.get("children") {
                Some(c) => (c, item.span.start.col + 2),
                None => {
                    return err(
                        "this folder has no \"children\" list; please edit the file by hand",
                    )
                }
            },
            Located::ShorthandItem { .. } => {
                return err("this favorite is a leaf; leaves cannot get children")
            }
        }
    };
    match &container.kind {
        NodeKind::Map { entries, flow: false } => {
            let children: Vec<&(Node, Node)> =
                entries.iter().filter(|(k, _)| !k.as_scalar().unwrap_or_default().starts_with('_')).collect();
            if children.iter().any(|(k, v)| compact_segment(k, v).as_deref() == Some(id.as_str())) {
                return err(format!("an entry with the ID \"{id}\" already exists here"));
            }
            if parent.is_empty() || !children.is_empty() {
                let indent = entries.first().map_or(container_indent, |(k, _)| k.span.start.col);
                Ok(insert_after_entries(src, entries, &fav.compact_lines(indent)))
            } else {
                err("the settings UI only adds favorites to folders")
            }
        }
        NodeKind::Null if parent.is_empty() => {
            // `favorites:` without content.
            Ok(insert_lines(src, after_line(&t, fkey.span.end.index), &fav.compact_lines(container_indent)))
        }
        NodeKind::Seq { items, flow: false } => {
            if items.iter().any(|i| i.as_map().is_none()) {
                return err("this list uses the short form; please edit the file by hand");
            }
            if items.iter().any(|i| i.get("id").and_then(Node::as_scalar).map(str::trim) == Some(id.as_str())) {
                return err(format!("an entry with the ID \"{id}\" already exists here"));
            }
            let indent = items.first().map_or(container_indent, |i| {
                // The "- " starts two columns before the item's first key.
                i.span.start.col.saturating_sub(2)
            });
            let Some(last) = items.last() else {
                return err("this list is empty; please edit the file by hand");
            };
            Ok(insert_lines(src, after_line(&t, last.span.end.index), &fav.explicit_lines(&id, indent)))
        }
        _ => err("this folder is written in a short form the settings UI cannot edit; please edit the file by hand"),
    }
}

/// Removes a favorite (and its children) from a favorites file.
pub fn remove_favorite(src: &str, path: &[&str]) -> Result<String, EditError> {
    if path.is_empty() {
        return err("nothing to remove");
    }
    let root = parse(src)?;
    let Some(favorites) = root.get("favorites") else {
        return err("the file has no favorites");
    };
    let t = Text::new(src);
    match locate(favorites, path)? {
        Located::Compact { key, value } => {
            if !matches!(favorites.kind, NodeKind::Map { flow: false, .. }) && path.len() == 1 {
                return err("this file uses a layout the settings UI cannot edit");
            }
            let start = t.line_start(t.byte(key.span.start.index));
            if src[start..t.byte(key.span.start.index)].trim().is_empty() {
                let end = after_line(&t, value.span.end.index.max(key.span.end.index));
                Ok(splice(src, start, end, ""))
            } else {
                err("this favorite is written in a short form; please edit the file by hand")
            }
        }
        Located::Explicit { item, .. } => {
            let first = t.byte(item.span.start.index);
            let start = t.line_start(first);
            let prefix = &src[start..first];
            if prefix.trim() != "-" {
                return err(
                    "this favorite is written in a short form; please edit the file by hand",
                );
            }
            Ok(splice(src, start, after_line(&t, item.span.end.index), ""))
        }
        Located::ShorthandItem { item, list } => {
            let NodeKind::Seq { items, flow } = &list.kind else {
                unreachable!()
            };
            if !flow {
                let start = t.line_start(t.byte(item.span.start.index));
                return Ok(splice(src, start, after_line(&t, item.span.end.index), ""));
            }
            if items.len() == 1 {
                return err("this is the only entry of the list; please edit the file by hand");
            }
            let pos = items.iter().position(|i| std::ptr::eq(i, item)).unwrap();
            let (start, end) = if pos + 1 < items.len() {
                (
                    t.byte(item.span.start.index),
                    t.byte(items[pos + 1].span.start.index),
                )
            } else {
                (
                    t.byte(items[pos - 1].span.end.index),
                    t.byte(item.span.end.index),
                )
            };
            Ok(splice(src, start, end, ""))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{parse_config, DEFAULT_CONFIG_YAML};
    use crate::favorites::{merge_parsed, parse_favorites_file};
    use std::path::PathBuf;

    #[test]
    fn scalars() {
        assert_eq!(scalar("Ctrl+Alt+Space"), "Ctrl+Alt+Space");
        assert_eq!(scalar("1h 23m"), "1h 23m");
        assert_eq!(scalar("08:00"), "\"08:00\"");
        assert_eq!(scalar("{title} · {elapsed}"), "\"{title} · {elapsed}\"");
        assert_eq!(scalar("true"), "\"true\"");
        assert_eq!(scalar("1.5"), "\"1.5\"");
        assert_eq!(scalar("http://localhost:5600"), "\"http://localhost:5600\"");
        assert_eq!(scalar(""), "\"\"");
        assert_eq!(scalar("Büro"), "Büro");
        assert_eq!(scalar("a \"b\""), "\"a \\\"b\\\"\"");
    }

    #[test]
    fn replaces_values_and_keeps_comments() {
        let src = "# top comment\nserver:\n  url: http://localhost:5600   # remote possible\n  heartbeat_interval: 60s\ntray:\n  display: template  # mode\n";
        let out = set_value(src, &["tray", "display"], &Value::Str("elapsed".into())).unwrap();
        assert_eq!(out, src.replace("display: template", "display: elapsed"));
        let out = set_value(
            &out,
            &["server", "url"],
            &Value::Str("http://aw:5600".into()),
        )
        .unwrap();
        assert!(out.contains("  url: \"http://aw:5600\"   # remote possible\n"));
        assert!(out.starts_with("# top comment\n"));
    }

    #[test]
    fn inserts_missing_keys_and_sections() {
        let src = "server:\n  url: http://localhost:5600\n\n# trailing comment\n";
        let out = set_value(
            src,
            &["server", "heartbeat_interval"],
            &Value::Str("30s".into()),
        )
        .unwrap();
        assert_eq!(out, "server:\n  url: http://localhost:5600\n  heartbeat_interval: 30s\n\n# trailing comment\n");
        let out = set_value(
            &out,
            &["popover", "scroll", "dwell_confirm"],
            &Value::Bool(true),
        )
        .unwrap();
        assert!(
            out.ends_with("  heartbeat_interval: 30s\npopover:\n  scroll:\n    dwell_confirm: true\n\n# trailing comment\n"),
            "{out}"
        );
        let c = parse_config(&out).unwrap();
        assert!(c.popover.scroll.dwell_confirm);
        assert_eq!(c.server.heartbeat_interval.as_secs(), 30);
    }

    #[test]
    fn empty_values_and_files() {
        let out = set_value("", &["tray", "color_icon"], &Value::Bool(false)).unwrap();
        assert_eq!(out, "tray:\n  color_icon: false\n");
        let out = set_value(
            "# just a comment",
            &["tray", "color_icon"],
            &Value::Bool(false),
        )
        .unwrap();
        assert_eq!(out, "# just a comment\ntray:\n  color_icon: false\n");
        let out = set_value(
            "hotkeys:\n  resume:\n  stop: Ctrl+Alt+S\n",
            &["hotkeys", "resume"],
            &Value::Str("Ctrl+Alt+R".into()),
        )
        .unwrap();
        assert_eq!(out, "hotkeys:\n  resume: Ctrl+Alt+R\n  stop: Ctrl+Alt+S\n");
        let out = set_value(
            "tray:\nserver:\n  url: x\n",
            &["tray", "display"],
            &Value::Str("icon".into()),
        )
        .unwrap();
        assert_eq!(out, "tray:\n  display: icon\nserver:\n  url: x\n");
        let out = set_value("tray:", &["tray", "display"], &Value::Str("icon".into())).unwrap();
        assert_eq!(out, "tray:\n  display: icon\n");
    }

    #[test]
    fn replaces_lists() {
        let src = "notifications:\n  work_hours:\n    days:\n      - mon\n      - tue\n    from: \"08:00\"\n";
        let out = set_value(
            src,
            &["notifications", "work_hours", "days"],
            &Value::List(vec!["mon".into(), "fri".into()]),
        )
        .unwrap();
        assert_eq!(
            out,
            "notifications:\n  work_hours:\n    days: [mon, fri]\n    from: \"08:00\"\n"
        );
        let out = set_value(
            &out,
            &["notifications", "work_hours", "days"],
            &Value::List(vec![]),
        )
        .unwrap();
        assert!(out.contains("days: []"));
    }

    #[test]
    fn every_setting_of_the_default_file_can_be_edited() {
        let mut src = DEFAULT_CONFIG_YAML.to_string();
        src = set_value(
            &src,
            &["server", "url"],
            &Value::Str("https://aw.example.com".into()),
        )
        .unwrap();
        src = set_value(
            &src,
            &["tray", "template"],
            &Value::Str("{path} – {elapsed}".into()),
        )
        .unwrap();
        src = set_value(
            &src,
            &["tray", "elapsed_format"],
            &Value::Str("1:23".into()),
        )
        .unwrap();
        src = set_value(
            &src,
            &["notifications", "idle_reminder"],
            &Value::Bool(false),
        )
        .unwrap();
        src = set_value(
            &src,
            &["notifications", "work_hours", "from"],
            &Value::Str("07:30".into()),
        )
        .unwrap();
        src = set_value(
            &src,
            &["popover", "scroll", "sensitivity"],
            &Value::Num(2.0),
        )
        .unwrap();
        src = set_value(&src, &["cli", "autostart_daemon"], &Value::Bool(false)).unwrap();
        let c = parse_config(&src).unwrap();
        assert_eq!(c.server.url, "https://aw.example.com");
        assert_eq!(c.tray.template, "{path} – {elapsed}");
        assert_eq!(c.tray.elapsed_format.as_str(), "1:23");
        assert_eq!(c.notifications.idle_reminder, None);
        assert_eq!(
            c.notifications.work_hours.from.format("%H:%M").to_string(),
            "07:30"
        );
        assert_eq!(c.popover.scroll.sensitivity, 2.0);
        assert!(!c.cli.autostart_daemon);
        // Comments survive.
        assert!(src.contains("# a remote aw-server is possible as well"));
        assert!(src.contains("# must be longer than dwell_time"));
    }

    #[test]
    fn flow_mappings_are_rejected() {
        assert!(set_value(
            "tray: {display: icon}\n",
            &["tray", "display"],
            &Value::Str("title".into())
        )
        .is_err());
        assert!(set_value(
            "tray: {display: icon}\n",
            &["tray", "color_icon"],
            &Value::Bool(true)
        )
        .is_err());
    }

    fn ids(src: &str) -> Vec<String> {
        let p = parse_favorites_file(src).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
        let (t, e) = merge_parsed(&[(PathBuf::from("x.yaml"), p)]);
        assert!(e.is_empty());
        t.all().iter().map(|f| f.id.clone()).collect()
    }

    fn fav(title: &str) -> NewFavorite {
        NewFavorite {
            title: title.into(),
            id: None,
            kind: None,
            color: None,
            icon: None,
        }
    }

    const COMPACT: &str = "# my favorites\nfavorites:\n  ACME:\n    _color: \"#E4572E\"   # client color\n    Website:\n      Development:\n        _icon: code\n      Code Review:\n    Tools: [Maintenance, Updates]\n  Break:\n    _kind: status\n\n# end\n";

    #[test]
    fn add_to_compact() {
        let out = add_favorite(
            COMPACT,
            &[],
            &NewFavorite {
                kind: Some(Kind::Status),
                icon: Some("coffee".into()),
                ..fav("Lunch")
            },
        )
        .unwrap();
        assert!(out.contains("  Break:\n    _kind: status\n  Lunch:\n    _icon: coffee\n    _kind: status\n\n# end\n"), "{out}");
        let out = add_favorite(
            &out,
            &["acme", "website"],
            &NewFavorite {
                id: Some("dep".into()),
                ..fav("Deployment")
            },
        )
        .unwrap();
        assert_eq!(
            ids(&out),
            [
                "acme",
                "acme/website",
                "acme/website/development",
                "acme/website/code-review",
                "acme/website/dep",
                "acme/tools",
                "acme/tools/maintenance",
                "acme/tools/updates",
                "break",
                "lunch"
            ]
        );
        assert!(out.contains("# client color"));
        // Duplicates and unsupported shapes are rejected.
        assert!(add_favorite(COMPACT, &[], &fav("break")).is_err());
        assert!(add_favorite(COMPACT, &["acme", "tools"], &fav("X")).is_err());
        assert!(add_favorite(COMPACT, &["nope"], &fav("X")).is_err());
        assert!(add_favorite(COMPACT, &["break"], &fav("X")).is_err());
        assert!(add_favorite(
            COMPACT,
            &[],
            &NewFavorite {
                color: Some("red".into()),
                ..fav("X")
            }
        )
        .is_err());
    }

    #[test]
    fn add_to_new_and_empty_files() {
        let out = add_favorite(
            "",
            &[],
            &NewFavorite {
                color: Some("#123456".into()),
                ..fav("Admin")
            },
        )
        .unwrap();
        assert_eq!(out, "favorites:\n  Admin:\n    _color: \"#123456\"\n");
        assert_eq!(ids(&out), ["admin"]);
        let out = add_favorite("favorites:\n", &[], &fav("A")).unwrap();
        assert_eq!(out, "favorites:\n  A:\n");
        let out = add_favorite("generated: false\n", &[], &fav("A")).unwrap();
        assert_eq!(ids(&out), ["a"]);
    }

    const EXPLICIT: &str = "generated: false\nfavorites:\n  - id: web\n    title: Web\n    children:\n      - id: web-1\n        title: \"WEB-1 Bug\"\n  - id: other\n    title: Other\n";

    #[test]
    fn add_to_explicit() {
        let out = add_favorite(EXPLICIT, &[], &fav("New")).unwrap();
        assert_eq!(ids(&out), ["web", "web/web-1", "other", "new"]);
        let out = add_favorite(
            &out,
            &["web"],
            &NewFavorite {
                id: Some("web-2".into()),
                ..fav("WEB-2: Footer")
            },
        )
        .unwrap();
        assert_eq!(ids(&out), ["web", "web/web-1", "web/web-2", "other", "new"]);
        assert!(out.contains("title: \"WEB-2: Footer\""));
        assert!(add_favorite(
            EXPLICIT,
            &[],
            &NewFavorite {
                id: Some("other".into()),
                ..fav("X")
            }
        )
        .is_err());
    }

    #[test]
    fn remove() {
        let out = remove_favorite(COMPACT, &["acme", "website", "development"]).unwrap();
        assert_eq!(
            ids(&out),
            [
                "acme",
                "acme/website",
                "acme/website/code-review",
                "acme/tools",
                "acme/tools/maintenance",
                "acme/tools/updates",
                "break"
            ]
        );
        let out = remove_favorite(COMPACT, &["acme"]).unwrap();
        assert_eq!(
            out,
            "# my favorites\nfavorites:\n  Break:\n    _kind: status\n\n# end\n"
        );
        let out = remove_favorite(COMPACT, &["break"]).unwrap();
        assert_eq!(ids(&out).last().unwrap(), "acme/tools/updates");
        assert!(out.ends_with("\n\n# end\n"));
        let out = remove_favorite(COMPACT, &["acme", "tools", "updates"]).unwrap();
        assert!(out.contains("Tools: [Maintenance]"), "{out}");
        let out = remove_favorite(COMPACT, &["acme", "tools", "maintenance"]).unwrap();
        assert!(out.contains("Tools: [Updates]"), "{out}");
        let out = remove_favorite(EXPLICIT, &["web", "web-1"]).unwrap();
        assert!(!out.contains("WEB-1"));
        let out = remove_favorite(EXPLICIT, &["other"]).unwrap();
        assert_eq!(ids(&out), ["web", "web/web-1"]);
        let out = remove_favorite(EXPLICIT, &["web"]).unwrap();
        assert_eq!(ids(&out), ["other"]);
        assert!(remove_favorite(COMPACT, &["nope"]).is_err());
        let block = "favorites:\n  Tools:\n    - A\n    - B\n";
        let out = remove_favorite(block, &["tools", "a"]).unwrap();
        assert_eq!(out, "favorites:\n  Tools:\n    - B\n");
    }

    #[test]
    fn remove_leaf_without_trailing_newline() {
        let out = remove_favorite("favorites:\n  A:\n  B:", &["b"]).unwrap();
        assert_eq!(out, "favorites:\n  A:\n");
        let out = remove_favorite("favorites:\n  A:\n    _icon: x\n  B:", &["a"]).unwrap();
        assert_eq!(out, "favorites:\n  B:");
    }
}
