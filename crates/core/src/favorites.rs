//! Favorites: parsing of `favorites.d/*.yaml` (compact and explicit notation),
//! merging of all files into one tree, and inheritance of color and icon.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::yaml::{self, Node, NodeKind, YamlError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Activity,
    Status,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "activity" => Some(Kind::Activity),
            "status" => Some(Kind::Status),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Activity => "activity",
            Kind::Status => "status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Notation {
    Compact,
    Explicit,
}

/// A favorite as defined in a single file, before merging.
#[derive(Debug, Clone, PartialEq)]
pub struct RawNode {
    /// The ID segment of this node (not the full ID).
    pub segment: String,
    pub title: String,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub kind: Option<Kind>,
    pub children: Vec<RawNode>,
    pub line: usize,
    /// Whether the ID was set explicitly (`_id` or explicit notation).
    pub explicit_id: bool,
}

/// The content of one favorites file.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFile {
    pub generated: bool,
    pub parent: Option<String>,
    pub parent_line: usize,
    pub notation: Notation,
    pub nodes: Vec<RawNode>,
}

/// Turns a title into an ID segment: "Code Review" → `code-review`.
pub fn slugify(title: &str) -> String {
    let ascii = deunicode::deunicode(title);
    let mut out = String::with_capacity(ascii.len());
    let mut dash = false;
    for c in ascii.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Checks an ID segment: letters, digits, `.`, `_`, `-`; must start with a letter or digit.
pub fn validate_segment(s: &str) -> Result<(), String> {
    let mut chars = s.chars();
    match chars.next() {
        None => return Err("ID must not be empty".into()),
        Some(c) if !c.is_ascii_alphanumeric() => {
            return Err(format!("ID \"{s}\" must start with a letter or digit"));
        }
        _ => {}
    }
    if let Some(c) = s
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "ID \"{s}\" contains \"{c}\"; only letters, digits, \".\", \"_\" and \"-\" are allowed"
        ));
    }
    Ok(())
}

/// Checks a full ID such as `acme/website/dev`.
pub fn validate_full_id(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("ID must not be empty".into());
    }
    for seg in s.split('/') {
        validate_segment(seg).map_err(|e| format!("invalid ID \"{s}\": {e}"))?;
    }
    Ok(())
}

pub fn validate_color(s: &str) -> Result<(), String> {
    let hex = s
        .strip_prefix('#')
        .ok_or_else(|| format!("color \"{s}\" must be a hex color like \"#E4572E\""))?;
    if !(hex.len() == 3 || hex.len() == 6) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "color \"{s}\" must be a hex color like \"#E4572E\" or \"#E52\""
        ));
    }
    Ok(())
}

struct Errs(Vec<YamlError>);

impl Errs {
    fn push(&mut self, node: &Node, msg: impl Into<String>) {
        self.0.push(YamlError::at(node, msg));
    }
}

const FILE_KEYS: &[&str] = &["favorites", "generated", "parent", "schema_version"];
const COMPACT_PROPS: &[&str] = &["_id", "_color", "_icon", "_kind"];
const EXPLICIT_FIELDS: &[&str] = &["id", "title", "color", "icon", "kind", "children"];

/// Parses one favorites file.
pub fn parse_favorites_file(src: &str) -> Result<ParsedFile, Vec<YamlError>> {
    let root = yaml::parse(src).map_err(|e| vec![e])?;
    let mut errs = Errs(Vec::new());
    let mut file = ParsedFile {
        generated: false,
        parent: None,
        parent_line: 0,
        notation: Notation::Compact,
        nodes: Vec::new(),
    };

    let entries = match &root.kind {
        NodeKind::Null => return Ok(file),
        NodeKind::Map { entries, .. } => entries,
        _ => {
            return Err(vec![YamlError::at(
                &root,
                "a favorites file must be a mapping with a \"favorites\" key",
            )])
        }
    };
    for (k, _) in entries {
        let key = k.as_scalar().unwrap_or_default();
        if !FILE_KEYS.contains(&key) {
            errs.push(
                k,
                format!(
                    "unknown key \"{key}\" (expected one of: {})",
                    FILE_KEYS.join(", ")
                ),
            );
        }
    }
    if let Some(n) = root.get("schema_version") {
        match n.as_i64() {
            Some(v) if (1..=crate::config::SCHEMA_VERSION).contains(&v) => {}
            _ => errs.push(
                n,
                format!(
                    "unsupported schema_version (this version supports up to {})",
                    crate::config::SCHEMA_VERSION
                ),
            ),
        }
    }
    if let Some(n) = root.get("generated") {
        match n.as_bool() {
            Some(b) => file.generated = b,
            None => errs.push(n, "\"generated\" must be true or false"),
        }
    }
    if let Some(n) = root.get("parent") {
        file.parent_line = n.line();
        match n.as_scalar() {
            Some(p) => {
                let p = p.trim().trim_matches('/').to_string();
                match validate_full_id(&p) {
                    Ok(()) => file.parent = Some(p),
                    Err(e) => errs.push(n, format!("parent: {e}")),
                }
            }
            None if n.is_null() => {}
            None => errs.push(
                n,
                "\"parent\" must be the full ID of a folder, e.g. acme/website",
            ),
        }
    }
    match root.get("favorites") {
        None => {
            errs.push(&root, "missing \"favorites\" key");
        }
        Some(fav) => match &fav.kind {
            NodeKind::Null => {}
            NodeKind::Map { entries, .. } => {
                file.notation = Notation::Compact;
                file.nodes = parse_compact_children(entries, true, &mut errs);
            }
            NodeKind::Seq { items, .. } => {
                file.notation = Notation::Explicit;
                file.nodes = parse_explicit_list(items, &mut errs);
            }
            NodeKind::Scalar { .. } => errs.push(
                fav,
                "\"favorites\" must be a mapping (compact notation) or a list (explicit notation)",
            ),
        },
    }

    if errs.0.is_empty() {
        Ok(file)
    } else {
        errs.0.sort_by_key(|e| (e.line, e.col));
        Err(errs.0)
    }
}

fn scalar_prop(key: &str, v: &Node, errs: &mut Errs) -> Option<String> {
    match &v.kind {
        NodeKind::Scalar { value, .. } if !value.trim().is_empty() => {
            Some(value.trim().to_string())
        }
        NodeKind::Null if key.ends_with("color") => {
            errs.push(v, format!("{key} has no value; colors must be quoted because # starts a comment, e.g. {key}: \"#E4572E\""));
            None
        }
        _ => {
            errs.push(v, format!("{key} must be a non-empty text value"));
            None
        }
    }
}

fn check_title(key: &Node, errs: &mut Errs) -> Option<String> {
    let title = key.as_scalar().unwrap_or_default();
    if title.trim().is_empty() {
        errs.push(key, "titles must not be empty");
        return None;
    }
    if title.starts_with('_') {
        errs.push(key, format!("titles must not start with \"_\" (\"{title}\"); properties are only allowed inside a favorite"));
        return None;
    }
    Some(title.to_string())
}

fn derive_segment(title: &str, id: Option<String>, at: &Node, errs: &mut Errs) -> Option<String> {
    match id {
        Some(id) => match validate_segment(&id) {
            Ok(()) => Some(id),
            Err(e) => {
                errs.push(at, e);
                None
            }
        },
        None => {
            let slug = slugify(title);
            if slug.is_empty() {
                errs.push(
                    at,
                    format!("cannot derive an ID from the title \"{title}\"; set _id"),
                );
                None
            } else {
                Some(slug)
            }
        }
    }
}

fn check_sibling_duplicates(
    nodes: &[RawNode],
    keys: &[&Node],
    errs: &mut Errs,
    notation: Notation,
) {
    let mut seen: HashMap<&str, &RawNode> = HashMap::new();
    for (node, key) in nodes.iter().zip(keys) {
        if let Some(first) = seen.get(node.segment.as_str()) {
            let msg = match notation {
                Notation::Compact => format!(
                    "\"{}\" and \"{}\" (line {}) result in the same ID \"{}\"; set _id on one of them",
                    node.title, first.title, first.line, node.segment
                ),
                Notation::Explicit => format!("duplicate id \"{}\" (first used on line {})", node.segment, first.line),
            };
            errs.push(key, msg);
        } else {
            seen.insert(&node.segment, node);
        }
    }
}

fn parse_compact_children(entries: &[(Node, Node)], top: bool, errs: &mut Errs) -> Vec<RawNode> {
    let mut nodes = Vec::new();
    let mut keys = Vec::new();
    for (k, v) in entries {
        if top {
            if let Some(t) = k.as_scalar() {
                if t.starts_with('_') {
                    errs.push(
                        k,
                        format!(
                            "properties like \"{t}\" are not allowed directly under \"favorites\""
                        ),
                    );
                    continue;
                }
            }
        }
        let Some(title) = check_title(k, errs) else {
            continue;
        };
        if let Some(node) = parse_compact_node(title, k, v, errs) {
            nodes.push(node);
            keys.push(k);
        }
    }
    check_sibling_duplicates(&nodes, &keys, errs, Notation::Compact);
    nodes
}

fn parse_compact_node(title: String, key: &Node, value: &Node, errs: &mut Errs) -> Option<RawNode> {
    let mut id = None;
    let mut color = None;
    let mut icon = None;
    let mut kind = None;
    let mut kind_node: Option<&Node> = None;
    let mut children = Vec::new();
    let mut ok = true;
    match &value.kind {
        NodeKind::Null => {}
        NodeKind::Map { entries, .. } => {
            let mut child_entries = Vec::new();
            for (k, v) in entries {
                let name = k.as_scalar().unwrap_or_default();
                if name.starts_with('_') {
                    if !COMPACT_PROPS.contains(&name) {
                        errs.push(
                            k,
                            format!(
                                "unknown property \"{name}\" (expected one of: {})",
                                COMPACT_PROPS.join(", ")
                            ),
                        );
                        ok = false;
                        continue;
                    }
                    let Some(val) = scalar_prop(name, v, errs) else {
                        ok = false;
                        continue;
                    };
                    match name {
                        "_id" => id = Some(val),
                        "_color" => match validate_color(&val) {
                            Ok(()) => color = Some(val),
                            Err(e) => {
                                errs.push(v, e);
                                ok = false;
                            }
                        },
                        "_icon" => icon = Some(val),
                        "_kind" => match Kind::parse(&val) {
                            Some(kd) => {
                                kind = Some(kd);
                                kind_node = Some(k);
                            }
                            None => {
                                errs.push(v, "_kind must be \"activity\" or \"status\"");
                                ok = false;
                            }
                        },
                        _ => unreachable!(),
                    }
                } else {
                    child_entries.push((k.clone(), v.clone()));
                }
            }
            children = parse_compact_children(&child_entries, false, errs);
        }
        NodeKind::Seq { items, .. } => {
            // List shorthand: children are leaves given by title.
            let mut keys = Vec::new();
            for item in items {
                match &item.kind {
                    NodeKind::Scalar { .. } => {
                        let Some(t) = check_title(item, errs) else { continue };
                        if let Some(seg) = derive_segment(&t, None, item, errs) {
                            children.push(RawNode {
                                segment: seg,
                                title: t,
                                color: None,
                                icon: None,
                                kind: None,
                                children: Vec::new(),
                                line: item.line(),
                                explicit_id: false,
                            });
                            keys.push(item);
                        }
                    }
                    _ => errs.push(
                        item,
                        "list shorthand items must be plain titles; use the mapping form to set properties",
                    ),
                }
            }
            check_sibling_duplicates(&children, &keys, errs, Notation::Compact);
            if items.is_empty() {
                errs.push(
                    value,
                    format!(
                        "\"{title}\" has an empty list; leave the value empty to define a leaf"
                    ),
                );
            }
        }
        NodeKind::Scalar { value: v, .. } => {
            errs.push(
                value,
                format!("unexpected value \"{v}\" for \"{title}\"; use an empty value for a leaf, or a mapping for properties and children"),
            );
            ok = false;
        }
    }
    if !children.is_empty() {
        if let Some(kn) = kind_node {
            errs.push(
                kn,
                format!("\"{title}\" is a folder; _kind is only allowed on leaves"),
            );
            ok = false;
        }
    }
    let explicit_id = id.is_some();
    let segment = derive_segment(&title, id, key, errs)?;
    ok.then_some(RawNode {
        segment,
        title,
        color,
        icon,
        kind,
        children,
        line: key.line(),
        explicit_id,
    })
}

fn parse_explicit_list(items: &[Node], errs: &mut Errs) -> Vec<RawNode> {
    let mut nodes = Vec::new();
    let mut keys = Vec::new();
    for item in items {
        if let Some(n) = parse_explicit_node(item, errs) {
            nodes.push(n);
            keys.push(item);
        }
    }
    check_sibling_duplicates(&nodes, &keys, errs, Notation::Explicit);
    nodes
}

fn parse_explicit_node(item: &Node, errs: &mut Errs) -> Option<RawNode> {
    let Some(entries) = item.as_map() else {
        errs.push(
            item,
            "each favorite must be a mapping with at least \"id\" and \"title\"",
        );
        return None;
    };
    let mut ok = true;
    for (k, _) in entries {
        let name = k.as_scalar().unwrap_or_default();
        if !EXPLICIT_FIELDS.contains(&name) {
            errs.push(
                k,
                format!(
                    "unknown field \"{name}\" (expected one of: {})",
                    EXPLICIT_FIELDS.join(", ")
                ),
            );
            ok = false;
        }
    }
    let field = |name: &str, errs: &mut Errs| -> Option<String> {
        item.get(name).and_then(|v| scalar_prop(name, v, errs))
    };
    let id = match item.get("id") {
        None => {
            errs.push(item, "missing \"id\"");
            None
        }
        Some(v) => scalar_prop("id", v, errs).and_then(|id| match validate_segment(&id) {
            Ok(()) => Some(id),
            Err(e) => {
                errs.push(v, e);
                None
            }
        }),
    };
    let title = match item.get("title") {
        None => {
            errs.push(item, "missing \"title\"");
            None
        }
        Some(_) => field("title", errs),
    };
    let color = field("color", errs);
    if let (Some(c), Some(n)) = (&color, item.get("color")) {
        if let Err(e) = validate_color(c) {
            errs.push(n, e);
            ok = false;
        }
    }
    let icon = field("icon", errs);
    let kind = match item.get("kind") {
        None => None,
        Some(n) => match field("kind", errs).as_deref().map(Kind::parse) {
            Some(Some(k)) => Some(k),
            _ => {
                errs.push(n, "kind must be \"activity\" or \"status\"");
                ok = false;
                None
            }
        },
    };
    let children = match item.get("children") {
        None => Vec::new(),
        Some(c) => match &c.kind {
            NodeKind::Null => Vec::new(),
            NodeKind::Seq { items, .. } => parse_explicit_list(items, errs),
            _ => {
                errs.push(c, "\"children\" must be a list");
                ok = false;
                Vec::new()
            }
        },
    };
    if !children.is_empty() && kind.is_some() {
        errs.push(
            item.get_key("kind").unwrap_or(item),
            "\"kind\" is only allowed on leaves, not on folders",
        );
        ok = false;
    }
    let (Some(id), Some(title)) = (id, title) else {
        return None;
    };
    ok.then_some(RawNode {
        segment: id,
        title,
        color,
        icon,
        kind,
        children,
        line: item.line(),
        explicit_id: true,
    })
}

// ---------------------------------------------------------------------------
// Merged tree
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueSource {
    Own,
    Inherited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolved {
    pub value: String,
    pub source: ValueSource,
}

/// A favorite in the merged tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Favorite {
    /// Full ID, e.g. `acme/website/dev`.
    pub id: String,
    pub title: String,
    /// Titles from the top level down to this favorite (inclusive).
    pub path: Vec<String>,
    /// `"folder"` or `"leaf"`.
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// Only set on leaves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    pub color: Option<Resolved>,
    pub icon: Option<Resolved>,
    pub file: PathBuf,
    pub line: usize,
    pub generated: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Favorite>,
    #[serde(skip)]
    own_color: Option<String>,
    #[serde(skip)]
    own_icon: Option<String>,
    /// Whether the ID was set explicitly (see `config check --strict`).
    #[serde(skip)]
    pub explicit_id: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Folder,
    Leaf,
}

impl Favorite {
    pub fn is_leaf(&self) -> bool {
        self.node_type == NodeType::Leaf
    }

    /// The effective kind of a leaf (`activity` by default).
    pub fn leaf_kind(&self) -> Kind {
        self.kind.unwrap_or(Kind::Activity)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FavoritesTree {
    pub roots: Vec<Favorite>,
}

impl FavoritesTree {
    pub fn find(&self, id: &str) -> Option<&Favorite> {
        let mut level = &self.roots;
        let mut found: Option<&Favorite> = None;
        let mut prefix = String::new();
        for seg in id.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(seg);
            let f = level.iter().find(|f| f.id == prefix)?;
            found = Some(f);
            level = &f.children;
        }
        found
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut Favorite> {
        let mut level = &mut self.roots;
        let mut prefix = String::new();
        let segs: Vec<&str> = id.split('/').collect();
        for (i, seg) in segs.iter().enumerate() {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(seg);
            let pos = level.iter().position(|f| f.id == prefix)?;
            if i == segs.len() - 1 {
                return Some(&mut level[pos]);
            }
            level = &mut level[pos].children;
        }
        None
    }

    /// All leaves in the fixed, configured order (depth-first).
    pub fn leaves(&self) -> Vec<&Favorite> {
        fn walk<'a>(nodes: &'a [Favorite], out: &mut Vec<&'a Favorite>) {
            for n in nodes {
                if n.is_leaf() {
                    out.push(n);
                } else {
                    walk(&n.children, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.roots, &mut out);
        out
    }

    /// All nodes (folders and leaves) depth-first.
    pub fn all(&self) -> Vec<&Favorite> {
        fn walk<'a>(nodes: &'a [Favorite], out: &mut Vec<&'a Favorite>) {
            for n in nodes {
                out.push(n);
                walk(&n.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.roots, &mut out);
        out
    }

    fn resolve_inheritance(&mut self) {
        fn walk(nodes: &mut [Favorite], color: Option<&str>, icon: Option<&str>) {
            for n in nodes {
                n.color = match (&n.own_color, color) {
                    (Some(c), _) => Some(Resolved {
                        value: c.clone(),
                        source: ValueSource::Own,
                    }),
                    (None, Some(c)) => Some(Resolved {
                        value: c.to_string(),
                        source: ValueSource::Inherited,
                    }),
                    (None, None) => None,
                };
                n.icon = match (&n.own_icon, icon) {
                    (Some(c), _) => Some(Resolved {
                        value: c.clone(),
                        source: ValueSource::Own,
                    }),
                    (None, Some(c)) => Some(Resolved {
                        value: c.to_string(),
                        source: ValueSource::Inherited,
                    }),
                    (None, None) => None,
                };
                let c = n.color.as_ref().map(|r| r.value.clone());
                let i = n.icon.as_ref().map(|r| r.value.clone());
                walk(&mut n.children, c.as_deref(), i.as_deref());
            }
        }
        walk(&mut self.roots, None, None);
    }
}

/// Merges files into a tree while keeping track of which file defined each ID.
struct Merger {
    tree: FavoritesTree,
    ids: HashMap<String, (PathBuf, usize)>,
}

impl Merger {
    fn new() -> Self {
        Merger {
            tree: FavoritesTree::default(),
            ids: HashMap::new(),
        }
    }

    /// Adds one file atomically: either all of its favorites or none.
    fn add(&mut self, file: &ParsedFile, path: &Path) -> Result<(), YamlError> {
        let (prefix, parent_path) = match &file.parent {
            None => (String::new(), Vec::new()),
            Some(p) => {
                let err = |msg: String| YamlError {
                    line: file.parent_line,
                    col: 0,
                    message: msg,
                };
                match self.tree.find(p) {
                    None => {
                        return Err(err(format!(
                            "parent \"{p}\" does not exist (it must be defined in a file that is loaded before this one)"
                        )))
                    }
                    Some(f) if f.is_leaf() => {
                        return Err(err(format!(
                            "parent \"{p}\" is a leaf; adding children would turn it into a folder"
                        )))
                    }
                    Some(f) => (format!("{p}/"), f.path.clone()),
                }
            }
        };

        // Collect all full IDs first and check for duplicates.
        fn collect(nodes: &[RawNode], prefix: &str, out: &mut Vec<(String, usize)>) {
            for n in nodes {
                let id = format!("{prefix}{}", n.segment);
                out.push((id.clone(), n.line));
                collect(&n.children, &format!("{id}/"), out);
            }
        }
        let mut new_ids = Vec::new();
        collect(&file.nodes, &prefix, &mut new_ids);
        for (id, line) in &new_ids {
            if let Some((other, other_line)) = self.ids.get(id) {
                return Err(YamlError {
                    line: *line,
                    col: 0,
                    message: format!(
                        "duplicate ID \"{id}\": already defined in {}:{other_line}; this file is ignored",
                        other.display()
                    ),
                });
            }
        }

        fn build(
            n: &RawNode,
            prefix: &str,
            parent_path: &[String],
            file: &ParsedFile,
            path: &Path,
        ) -> Favorite {
            let id = format!("{prefix}{}", n.segment);
            let mut p = parent_path.to_vec();
            p.push(n.title.clone());
            let children: Vec<Favorite> = n
                .children
                .iter()
                .map(|c| build(c, &format!("{id}/"), &p, file, path))
                .collect();
            let leaf = children.is_empty();
            Favorite {
                id,
                title: n.title.clone(),
                path: p,
                node_type: if leaf {
                    NodeType::Leaf
                } else {
                    NodeType::Folder
                },
                kind: if leaf {
                    Some(n.kind.unwrap_or(Kind::Activity))
                } else {
                    None
                },
                color: None,
                icon: None,
                file: path.to_path_buf(),
                line: n.line,
                generated: file.generated,
                children,
                own_color: n.color.clone(),
                own_icon: n.icon.clone(),
                explicit_id: n.explicit_id,
            }
        }
        let built: Vec<Favorite> = file
            .nodes
            .iter()
            .map(|n| build(n, &prefix, &parent_path, file, path))
            .collect();
        match &file.parent {
            None => self.tree.roots.extend(built),
            Some(p) => self
                .tree
                .find_mut(p)
                .expect("parent checked above")
                .children
                .extend(built),
        }
        for (id, line) in new_ids {
            self.ids.insert(id, (path.to_path_buf(), line));
        }
        Ok(())
    }
}

/// Status of one favorites file after loading.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum FileStatus {
    Ok,
    /// The file is invalid and ignored.
    Invalid {
        errors: Vec<FileError>,
    },
    /// The current content is invalid; the last valid version stays active.
    LastValid {
        errors: Vec<FileError>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileError {
    pub line: Option<usize>,
    pub message: String,
}

impl From<YamlError> for FileError {
    fn from(e: YamlError) -> Self {
        FileError {
            line: (e.line > 0).then_some(e.line),
            message: e.message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileReport {
    pub path: PathBuf,
    pub generated: bool,
    pub notation: Option<Notation>,
    #[serde(flatten)]
    pub status: FileStatus,
}

/// Remembers the last valid version of each file (see "errors are isolated per file").
#[derive(Debug, Default, Clone)]
pub struct FavoritesCache {
    last_valid: HashMap<PathBuf, (String, ParsedFile)>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FavoritesLoad {
    pub tree: FavoritesTree,
    pub files: Vec<FileReport>,
    /// Problems that don't belong to a single file (e.g. a missing directory).
    pub warnings: Vec<String>,
}

impl FavoritesLoad {
    pub fn has_errors(&self) -> bool {
        self.files.iter().any(|f| f.status != FileStatus::Ok)
    }
}

/// Whether a directory entry should be loaded as a favorites file.
pub fn is_favorites_file(name: &str) -> bool {
    if name.starts_with('.') || name.ends_with('~') || name.ends_with(".tmp") {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".yaml") || lower.ends_with(".yml")
}

/// Lists the favorites files of the given directories in load order.
pub fn list_files(dirs: &[PathBuf], warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen_dirs = HashSet::new();
    for dir in dirs {
        if !seen_dirs.insert(dir.clone()) {
            continue;
        }
        let rd = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                warnings.push(format!(
                    "cannot read favorites folder {}: {e}",
                    dir.display()
                ));
                continue;
            }
        };
        let mut files: Vec<(String, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                let path = e.path();
                (is_favorites_file(&name)
                    && std::fs::metadata(&path)
                        .map(|m| m.is_file())
                        .unwrap_or(false))
                .then_some((name, path))
            })
            .collect();
        files.sort();
        out.extend(files.into_iter().map(|(_, p)| p));
    }
    out
}

/// Loads and merges all favorites files. With a cache, the last valid version
/// of an invalid file stays active.
pub fn load_favorites(dirs: &[PathBuf], mut cache: Option<&mut FavoritesCache>) -> FavoritesLoad {
    let mut warnings = Vec::new();
    let files = list_files(dirs, &mut warnings);
    let mut merger = Merger::new();
    let mut reports = Vec::new();

    for path in &files {
        let src = std::fs::read_to_string(path);
        let (current_errors, notation, generated): (Vec<FileError>, Option<Notation>, bool) =
            match &src {
                Err(e) => (
                    vec![FileError {
                        line: None,
                        message: format!("cannot read file: {e}"),
                    }],
                    None,
                    false,
                ),
                Ok(src) => match parse_favorites_file(src) {
                    Err(errs) => (errs.into_iter().map(FileError::from).collect(), None, false),
                    Ok(parsed) => match merger.add(&parsed, path) {
                        Ok(()) => {
                            if let Some(c) = cache.as_deref_mut() {
                                c.last_valid
                                    .insert(path.clone(), (src.clone(), parsed.clone()));
                            }
                            reports.push(FileReport {
                                path: path.clone(),
                                generated: parsed.generated,
                                notation: Some(parsed.notation),
                                status: FileStatus::Ok,
                            });
                            continue;
                        }
                        Err(e) => (vec![e.into()], Some(parsed.notation), parsed.generated),
                    },
                },
            };
        // Fall back to the last valid version, if it differs from the current one.
        let fallback = cache
            .as_deref()
            .and_then(|c| c.last_valid.get(path))
            .filter(|(old_src, _)| match &src {
                Ok(s) => s != old_src,
                Err(_) => true,
            });
        let status = match fallback {
            Some((_, old)) => match merger.add(old, path) {
                Ok(()) => FileStatus::LastValid {
                    errors: current_errors,
                },
                Err(e) => {
                    let mut errors = current_errors;
                    errors.push(FileError {
                        line: None,
                        message: format!(
                            "the last valid version cannot be used either: {}",
                            e.message
                        ),
                    });
                    FileStatus::Invalid { errors }
                }
            },
            None => FileStatus::Invalid {
                errors: current_errors,
            },
        };
        let (generated, notation) = match (&status, fallback) {
            (FileStatus::LastValid { .. }, Some((_, old))) => (old.generated, Some(old.notation)),
            _ => (generated, notation),
        };
        reports.push(FileReport {
            path: path.clone(),
            generated,
            notation,
            status,
        });
    }

    if let Some(c) = cache {
        let present: HashSet<&PathBuf> = files.iter().collect();
        c.last_valid.retain(|p, _| present.contains(p));
    }

    merger.tree.resolve_inheritance();
    FavoritesLoad {
        tree: merger.tree,
        files: reports,
        warnings,
    }
}

/// Merges already parsed files (used in tests and by the settings UI to validate edits).
pub fn merge_parsed(files: &[(PathBuf, ParsedFile)]) -> (FavoritesTree, Vec<(PathBuf, YamlError)>) {
    let mut merger = Merger::new();
    let mut errors = Vec::new();
    for (path, f) in files {
        if let Err(e) = merger.add(f, path) {
            errors.push((path.clone(), e));
        }
    }
    merger.tree.resolve_inheritance();
    (merger.tree, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ParsedFile {
        parse_favorites_file(src).unwrap_or_else(|e| panic!("{e:?}"))
    }

    fn errs(src: &str) -> Vec<YamlError> {
        parse_favorites_file(src).expect_err("expected an error")
    }

    fn tree(files: &[(&str, &str)]) -> (FavoritesTree, Vec<(PathBuf, YamlError)>) {
        let parsed: Vec<(PathBuf, ParsedFile)> = files
            .iter()
            .map(|(n, s)| (PathBuf::from(n), parse(s)))
            .collect();
        merge_parsed(&parsed)
    }

    const CLIENTS: &str = r##"
favorites:
  ACME:
    _color: "#E4572E"            # applies to ACME and everything below it
    _icon: briefcase
    Website:
      Development:
        _icon: code              # overrides the inherited icon
      Code Review:               # empty value = leaf with inherited color/icon
    Internal Tools:
      _color: "#3B82F6"          # overrides the inherited color for this subtree
      _id: tools                 # explicit ID instead of "internal-tools"
      Maintenance:
  Break:
    _kind: status
    _icon: coffee
"##;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Code Review"), "code-review");
        assert_eq!(slugify("ACME"), "acme");
        assert_eq!(slugify("WEB-123: Checkout bug"), "web-123-checkout-bug");
        assert_eq!(slugify("  Büro & Verwaltung  "), "buro-verwaltung");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn compact_spec_example() {
        let (t, e) = tree(&[("10-clients.yaml", CLIENTS)]);
        assert!(e.is_empty());
        let ids: Vec<&str> = t.all().iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "acme",
                "acme/website",
                "acme/website/development",
                "acme/website/code-review",
                "acme/tools",
                "acme/tools/maintenance",
                "break"
            ]
        );
        let dev = t.find("acme/website/development").unwrap();
        assert_eq!(
            dev.color,
            Some(Resolved {
                value: "#E4572E".into(),
                source: ValueSource::Inherited
            })
        );
        assert_eq!(
            dev.icon,
            Some(Resolved {
                value: "code".into(),
                source: ValueSource::Own
            })
        );
        assert_eq!(dev.path, ["ACME", "Website", "Development"]);
        assert_eq!(dev.kind, Some(Kind::Activity));
        let cr = t.find("acme/website/code-review").unwrap();
        assert_eq!(cr.icon.as_ref().unwrap().value, "briefcase");
        let m = t.find("acme/tools/maintenance").unwrap();
        assert_eq!(m.color.as_ref().unwrap().value, "#3B82F6");
        let tools = t.find("acme/tools").unwrap();
        assert_eq!(tools.color.as_ref().unwrap().source, ValueSource::Own);
        assert!(!tools.is_leaf());
        assert_eq!(tools.kind, None);
        let brk = t.find("break").unwrap();
        assert_eq!(brk.kind, Some(Kind::Status));
        assert!(brk.color.is_none());
        let leaves: Vec<&str> = t.leaves().iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            leaves,
            [
                "acme/website/development",
                "acme/website/code-review",
                "acme/tools/maintenance",
                "break"
            ]
        );
    }

    #[test]
    fn list_shorthand() {
        let src = "favorites:\n  ACME:\n    _color: \"#E4572E\"\n    Website: [Development, Code Review, Deployment]\n";
        let (t, _) = tree(&[("a.yaml", src)]);
        let leaves: Vec<&str> = t.leaves().iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            leaves,
            [
                "acme/website/development",
                "acme/website/code-review",
                "acme/website/deployment"
            ]
        );
        assert_eq!(
            t.find("acme/website/deployment")
                .unwrap()
                .color
                .as_ref()
                .unwrap()
                .value,
            "#E4572E"
        );
    }

    #[test]
    fn explicit_with_parent() {
        let jira = r#"
# generated by jira-sync – do not edit by hand
generated: true
parent: acme/website
favorites:
  - id: web-123
    title: "WEB-123 Checkout bug"
  - id: web-140
    title: "WEB-140 New footer"
    icon: bug
    children: []
"#;
        let (t, e) = tree(&[("10-clients.yaml", CLIENTS), ("50-jira.yaml", jira)]);
        assert!(e.is_empty(), "{e:?}");
        let website = t.find("acme/website").unwrap();
        let ids: Vec<&str> = website.children.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "acme/website/development",
                "acme/website/code-review",
                "acme/website/web-123",
                "acme/website/web-140"
            ]
        );
        let w123 = t.find("acme/website/web-123").unwrap();
        assert!(w123.generated);
        assert!(w123.is_leaf());
        assert_eq!(w123.path, ["ACME", "Website", "WEB-123 Checkout bug"]);
        assert_eq!(w123.color.as_ref().unwrap().value, "#E4572E");
        assert_eq!(w123.icon.as_ref().unwrap().source, ValueSource::Inherited);
        assert_eq!(
            t.find("acme/website/web-140")
                .unwrap()
                .icon
                .as_ref()
                .unwrap()
                .source,
            ValueSource::Own
        );
    }

    #[test]
    fn compact_with_parent() {
        let extra = "parent: acme\nfavorites:\n  Support:\n";
        let (t, e) = tree(&[("10.yaml", CLIENTS), ("20.yaml", extra)]);
        assert!(e.is_empty());
        let acme = t.find("acme").unwrap();
        assert_eq!(acme.children.last().unwrap().id, "acme/support");
    }

    #[test]
    fn explicit_nested() {
        let src = "favorites:\n  - id: a\n    title: A\n    color: \"#123\"\n    children:\n      - id: b\n        title: B\n        kind: status\n";
        let (t, _) = tree(&[("x.yaml", src)]);
        let b = t.find("a/b").unwrap();
        assert_eq!(b.kind, Some(Kind::Status));
        assert_eq!(b.color.as_ref().unwrap().value, "#123");
    }

    #[test]
    fn duplicate_ids_across_files_reject_later_file() {
        let a = "favorites:\n  ACME:\n    Dev:\n";
        let b = "favorites:\n  ACME:\n    Other:\n  Unrelated:\n";
        let (t, e) = tree(&[("10.yaml", a), ("20.yaml", b)]);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, PathBuf::from("20.yaml"));
        assert!(e[0].1.message.contains("duplicate ID \"acme\""));
        assert!(e[0].1.message.contains("10.yaml"));
        // The whole later file is rejected, including unrelated entries.
        assert!(t.find("unrelated").is_none());
        assert!(t.find("acme/dev").is_some());
    }

    #[test]
    fn parent_errors() {
        let missing = "parent: nope\nfavorites:\n  X:\n";
        let (_, e) = tree(&[("10.yaml", CLIENTS), ("20.yaml", missing)]);
        assert!(e[0].1.message.contains("does not exist"));
        assert_eq!(e[0].1.line, 1);
        let leaf = "parent: break\nfavorites:\n  X:\n";
        let (t, e) = tree(&[("10.yaml", CLIENTS), ("20.yaml", leaf)]);
        assert!(e[0].1.message.contains("is a leaf"));
        assert!(t.find("break").unwrap().is_leaf());
    }

    #[test]
    fn compact_validation_errors() {
        let e = errs("favorites:\n  A:\n    _colr: \"#fff\"\n");
        assert!(e[0].message.contains("unknown property \"_colr\""));
        assert_eq!(e[0].line, 3);

        let e = errs("favorites:\n  A:\n    _color: #E4572E\n");
        assert!(e[0].message.contains("quoted"), "{}", e[0].message);

        let e = errs("favorites:\n  A:\n    _kind: status\n    B:\n");
        assert!(e[0].message.contains("folder"));

        let e = errs("favorites:\n  Code Review:\n  code-review:\n");
        assert!(e[0].message.contains("same ID"));
        assert_eq!(e[0].line, 3);

        let e = errs("favorites:\n  A:\n  A:\n");
        assert!(e[0].message.contains("duplicate key"));

        let e = errs("favorites:\n  _color: \"#fff\"\n");
        assert!(e[0].message.contains("not allowed directly"));

        let e = errs("favorites:\n  A: hello\n");
        assert!(e[0].message.contains("unexpected value"));

        let e = errs("favorites:\n  A:\n    _id: \"bad id\"\n");
        assert!(e[0].message.contains("contains"));

        let e = errs("favorites:\n  A:\n    _color: red\n");
        assert!(e[0].message.contains("hex color"));

        let e = errs("favorites:\n  \"!!!\":\n");
        assert!(e[0].message.contains("set _id"));

        let e = errs("favorites:\n  A: [x, {y: 1}]\n");
        assert!(e[0].message.contains("plain titles"));

        let e = errs("favourites:\n  A:\n");
        assert!(e
            .iter()
            .any(|x| x.message.contains("unknown key \"favourites\"")));
    }

    #[test]
    fn explicit_validation_errors() {
        let e = errs("favorites:\n  - title: A\n");
        assert!(e[0].message.contains("missing \"id\""));
        let e = errs("favorites:\n  - id: a\n");
        assert!(e[0].message.contains("missing \"title\""));
        let e = errs("favorites:\n  - id: a\n    title: A\n  - id: a\n    title: B\n");
        assert!(e[0].message.contains("duplicate id"));
        let e = errs("favorites:\n  - id: a\n    title: A\n    colour: x\n");
        assert!(e[0].message.contains("unknown field"));
        let e = errs("favorites:\n  - id: a\n    title: A\n    kind: status\n    children:\n      - id: b\n        title: B\n");
        assert!(e[0].message.contains("only allowed on leaves"));
        let e = errs("favorites:\n  - id: a/b\n    title: A\n");
        assert!(e[0].message.contains("contains"));
        let e = errs("favorites:\n  - just a string\n");
        assert!(e[0].message.contains("must be a mapping"));
    }

    #[test]
    fn empty_and_generated() {
        let p = parse("");
        assert!(p.nodes.is_empty());
        let p = parse("favorites:\n");
        assert!(p.nodes.is_empty());
        let p = parse("generated: true\nfavorites: []\n");
        assert!(p.generated);
        assert_eq!(p.notation, Notation::Explicit);
        assert!(parse_favorites_file("generated: true\n").is_err());
    }

    #[test]
    fn titles_with_special_characters() {
        let (t, _) = tree(&[("a.yaml", "favorites:\n  \"WEB-123: Checkout bug\":\n")]);
        let f = t.find("web-123-checkout-bug").unwrap();
        assert_eq!(f.title, "WEB-123: Checkout bug");
    }

    #[test]
    fn leaf_with_only_properties() {
        let (t, _) = tree(&[("a.yaml", "favorites:\n  Development: {_icon: code}\n")]);
        let f = t.find("development").unwrap();
        assert!(f.is_leaf());
        assert_eq!(f.icon.as_ref().unwrap().value, "code");
    }

    #[test]
    fn file_filter() {
        assert!(is_favorites_file("10-clients.yaml"));
        assert!(is_favorites_file("x.YML"));
        assert!(!is_favorites_file(".hidden.yaml"));
        assert!(!is_favorites_file("x.yaml~"));
        assert!(!is_favorites_file("x.yaml.tmp"));
        assert!(!is_favorites_file("x.tmp"));
        assert!(!is_favorites_file("x.json"));
    }

    #[test]
    fn load_from_disk_with_cache() {
        let dir = tempdir();
        let fav = dir.join("favorites.d");
        std::fs::create_dir_all(fav.join("sub")).unwrap();
        std::fs::write(fav.join("20-b.yaml"), "favorites:\n  B:\n").unwrap();
        std::fs::write(fav.join("10-a.yaml"), "favorites:\n  A:\n").unwrap();
        std::fs::write(fav.join(".hidden.yaml"), "favorites:\n  H:\n").unwrap();
        std::fs::write(fav.join("sub/30-c.yaml"), "favorites:\n  C:\n").unwrap();
        std::fs::write(fav.join("x.yaml~"), "favorites:\n  T:\n").unwrap();
        let mut cache = FavoritesCache::default();
        let l = load_favorites(std::slice::from_ref(&fav), Some(&mut cache));
        let ids: Vec<&str> = l.tree.roots.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert!(!l.has_errors());

        // Break a file: its last valid version stays active.
        std::fs::write(fav.join("10-a.yaml"), "favorites:\n  A:\n    _bad: 1\n").unwrap();
        let l = load_favorites(std::slice::from_ref(&fav), Some(&mut cache));
        let ids: Vec<&str> = l.tree.roots.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert!(matches!(l.files[0].status, FileStatus::LastValid { .. }));

        // Without cache the file is skipped; other files keep working.
        let l = load_favorites(std::slice::from_ref(&fav), None);
        let ids: Vec<&str> = l.tree.roots.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["b"]);
        assert!(matches!(l.files[0].status, FileStatus::Invalid { .. }));
        assert_eq!(l.files[0].path, fav.join("10-a.yaml"));

        // Deleting the file removes it (and its cache entry).
        std::fs::remove_file(fav.join("10-a.yaml")).unwrap();
        let l = load_favorites(std::slice::from_ref(&fav), Some(&mut cache));
        assert_eq!(l.tree.roots.len(), 1);
        assert!(!cache.last_valid.contains_key(&fav.join("10-a.yaml")));

        // Missing directories are ignored.
        let l = load_favorites(&[dir.join("nope"), fav.clone()], None);
        assert_eq!(l.tree.roots.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn duplicate_falls_back_to_last_valid() {
        let dir = tempdir();
        std::fs::write(dir.join("10-a.yaml"), "favorites:\n  A:\n").unwrap();
        std::fs::write(dir.join("20-b.yaml"), "favorites:\n  B:\n").unwrap();
        let mut cache = FavoritesCache::default();
        load_favorites(std::slice::from_ref(&dir), Some(&mut cache));
        std::fs::write(dir.join("20-b.yaml"), "favorites:\n  B:\n  A:\n").unwrap();
        let l = load_favorites(std::slice::from_ref(&dir), Some(&mut cache));
        let ids: Vec<&str> = l.tree.roots.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert!(matches!(l.files[1].status, FileStatus::LastValid { .. }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn multiple_dirs_order() {
        let dir = tempdir();
        let d1 = dir.join("one");
        let d2 = dir.join("two");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::write(d1.join("90-z.yaml"), "favorites:\n  Z:\n").unwrap();
        std::fs::write(d2.join("10-a.yaml"), "favorites:\n  A:\n").unwrap();
        let l = load_favorites(&[d1, d2], None);
        let ids: Vec<&str> = l.tree.roots.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["z", "a"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    pub(crate) fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "aw-manual-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
