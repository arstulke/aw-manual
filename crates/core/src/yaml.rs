//! A small YAML document model that keeps source positions and mapping order.
//!
//! Configuration and favorites files are parsed into [`Node`] trees instead of
//! deserializing them directly, because we need
//! - mapping order (the order of favorites is the written order),
//! - line numbers for every validation error,
//! - detection of duplicate keys, and
//! - exact source spans for comment-preserving edits (see [`crate::yaml_edit`]).

use std::fmt;

use saphyr_parser::{Event, Parser, ScalarStyle, Span};

/// A position in the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    /// 1-based line number.
    pub line: usize,
    /// 0-based column (in characters).
    pub col: usize,
    /// Offset in characters from the start of the input.
    pub index: usize,
}

impl From<saphyr_parser::Marker> for Pos {
    fn from(m: saphyr_parser::Marker) -> Self {
        Pos {
            line: m.line(),
            col: m.col(),
            index: m.index(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeSpan {
    pub start: Pos,
    pub end: Pos,
}

impl From<Span> for NodeSpan {
    fn from(s: Span) -> Self {
        NodeSpan {
            start: s.start.into(),
            end: s.end.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Quoted,
    Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// `~`, `null`, or an empty value.
    Null,
    Scalar {
        value: String,
        style: Style,
    },
    Seq {
        items: Vec<Node>,
        flow: bool,
    },
    Map {
        entries: Vec<(Node, Node)>,
        flow: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub span: NodeSpan,
}

/// A parse or validation error with a source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl YamlError {
    pub fn at(node: &Node, message: impl Into<String>) -> Self {
        YamlError {
            line: node.span.start.line,
            col: node.span.start.col,
            message: message.into(),
        }
    }
}

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for YamlError {}

impl Node {
    pub fn is_null(&self) -> bool {
        matches!(self.kind, NodeKind::Null)
    }

    pub fn line(&self) -> usize {
        self.span.start.line
    }

    /// Short description of the node type, used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self.kind {
            NodeKind::Null => "an empty value",
            NodeKind::Scalar { .. } => "a scalar",
            NodeKind::Seq { .. } => "a list",
            NodeKind::Map { .. } => "a mapping",
        }
    }

    /// The text of a scalar (any style), `None` for other node types.
    pub fn as_scalar(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Scalar { value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(Node, Node)]> {
        match &self.kind {
            NodeKind::Map { entries, .. } => Some(entries),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> Option<&[Node]> {
        match &self.kind {
            NodeKind::Seq { items, .. } => Some(items),
            _ => None,
        }
    }

    /// Looks up a key in a mapping node.
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.as_map()?
            .iter()
            .find(|(k, _)| k.as_scalar() == Some(key))
            .map(|(_, v)| v)
    }

    /// The key node for `key` in a mapping node.
    pub fn get_key(&self, key: &str) -> Option<&Node> {
        self.as_map()?
            .iter()
            .find(|(k, _)| k.as_scalar() == Some(key))
            .map(|(k, _)| k)
    }

    /// Interprets a scalar as a YAML 1.2 boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match &self.kind {
            NodeKind::Scalar {
                value,
                style: Style::Plain,
            } => match value.as_str() {
                "true" | "True" | "TRUE" => Some(true),
                "false" | "False" | "FALSE" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// Interprets a plain scalar as a number.
    pub fn as_f64(&self) -> Option<f64> {
        match &self.kind {
            NodeKind::Scalar {
                value,
                style: Style::Plain,
            } => {
                let v: f64 = value.parse().ok()?;
                v.is_finite().then_some(v)
            }
            _ => None,
        }
    }

    /// Interprets a plain scalar as an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match &self.kind {
            NodeKind::Scalar {
                value,
                style: Style::Plain,
            } => value.parse().ok(),
            _ => None,
        }
    }
}

fn is_null_scalar(value: &str, style: ScalarStyle) -> bool {
    style == ScalarStyle::Plain && matches!(value, "" | "~" | "null" | "Null" | "NULL")
}

enum Frame {
    Seq {
        items: Vec<Node>,
        start: NodeSpan,
        flow: bool,
    },
    Map {
        entries: Vec<(Node, Node)>,
        pending_key: Option<Node>,
        start: NodeSpan,
        flow: bool,
    },
}

/// Parses a YAML document. An empty input yields a `Null` node.
pub fn parse(src: &str) -> Result<Node, YamlError> {
    let mut parser = Parser::new_from_str(src);
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<Node> = None;
    let mut documents = 0usize;
    let mut last_end = Pos::default();

    while let Some(item) = parser.next_event() {
        let (event, span) = item.map_err(|e| YamlError {
            line: e.marker().line(),
            col: e.marker().col(),
            message: e.info().to_string(),
        })?;
        let span: NodeSpan = span.into();
        if span.end.index > last_end.index {
            last_end = span.end;
        }
        let finished: Option<Node> = match event {
            Event::StreamStart | Event::StreamEnd | Event::DocumentEnd | Event::Nothing => None,
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(YamlError {
                        line: span.start.line,
                        col: span.start.col,
                        message: "multiple YAML documents in one file are not supported".into(),
                    });
                }
                None
            }
            Event::Alias(_) => {
                return Err(YamlError {
                    line: span.start.line,
                    col: span.start.col,
                    message: "YAML aliases (*name) are not supported".into(),
                });
            }
            Event::Scalar(value, style, _, _) => {
                let kind = if is_null_scalar(&value, style) {
                    NodeKind::Null
                } else {
                    let style = match style {
                        ScalarStyle::Plain => Style::Plain,
                        ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => Style::Quoted,
                        ScalarStyle::Literal | ScalarStyle::Folded => Style::Block,
                    };
                    NodeKind::Scalar {
                        value: value.into_owned(),
                        style,
                    }
                };
                Some(Node { kind, span })
            }
            Event::SequenceStart(_, _) => {
                stack.push(Frame::Seq {
                    items: Vec::new(),
                    start: span,
                    flow: span.end.index > span.start.index,
                });
                None
            }
            Event::MappingStart(_, _) => {
                stack.push(Frame::Map {
                    entries: Vec::new(),
                    pending_key: None,
                    start: span,
                    flow: span.end.index > span.start.index,
                });
                None
            }
            Event::SequenceEnd => match stack.pop() {
                Some(Frame::Seq { items, start, flow }) => {
                    let end = if flow {
                        span.end
                    } else {
                        items.last().map_or(start.end, |n| n.span.end)
                    };
                    Some(Node {
                        kind: NodeKind::Seq { items, flow },
                        span: NodeSpan {
                            start: start.start,
                            end,
                        },
                    })
                }
                _ => unreachable!("parser emitted unbalanced sequence end"),
            },
            Event::MappingEnd => match stack.pop() {
                Some(Frame::Map {
                    entries,
                    start,
                    flow,
                    ..
                }) => {
                    let end = if flow {
                        span.end
                    } else {
                        entries.last().map_or(start.end, |(_, v)| v.span.end)
                    };
                    Some(Node {
                        kind: NodeKind::Map { entries, flow },
                        span: NodeSpan {
                            start: start.start,
                            end,
                        },
                    })
                }
                _ => unreachable!("parser emitted unbalanced mapping end"),
            },
        };

        if let Some(node) = finished {
            match stack.last_mut() {
                None => root = Some(node),
                Some(Frame::Seq { items, .. }) => items.push(node),
                Some(Frame::Map {
                    entries,
                    pending_key,
                    ..
                }) => match pending_key.take() {
                    None => {
                        let key_text = match &node.kind {
                            NodeKind::Scalar { value, .. } => value.clone(),
                            NodeKind::Null => {
                                return Err(YamlError::at(
                                    &node,
                                    "empty mapping keys are not allowed",
                                ));
                            }
                            _ => {
                                return Err(YamlError::at(&node, "mapping keys must be plain text"))
                            }
                        };
                        if let Some((first, _)) = entries
                            .iter()
                            .find(|(k, _)| k.as_scalar() == Some(&key_text))
                        {
                            return Err(YamlError::at(
                                &node,
                                format!(
                                    "duplicate key \"{}\" (first defined on line {})",
                                    key_text,
                                    first.line()
                                ),
                            ));
                        }
                        *pending_key = Some(node);
                    }
                    Some(key) => entries.push((key, node)),
                },
            }
        }
    }

    Ok(root.unwrap_or(Node {
        kind: NodeKind::Null,
        span: NodeSpan {
            start: last_end,
            end: last_end,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_lines() {
        let n = parse("b: 1\na: 2\nc:\n  - x\n  - y\n").unwrap();
        let keys: Vec<_> = n
            .as_map()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_scalar().unwrap())
            .collect();
        assert_eq!(keys, ["b", "a", "c"]);
        assert_eq!(n.get("a").unwrap().line(), 2);
        assert_eq!(n.get("c").unwrap().as_seq().unwrap()[1].line(), 5);
    }

    #[test]
    fn null_values() {
        let n = parse("a:\nb: ~\nc: null\nd: \"\"\n").unwrap();
        assert!(n.get("a").unwrap().is_null());
        assert!(n.get("b").unwrap().is_null());
        assert!(n.get("c").unwrap().is_null());
        assert_eq!(n.get("d").unwrap().as_scalar(), Some(""));
    }

    #[test]
    fn empty_document() {
        assert!(parse("").unwrap().is_null());
        assert!(parse("# only a comment\n").unwrap().is_null());
    }

    #[test]
    fn duplicate_keys_rejected() {
        let e = parse("a: 1\nb: 2\na: 3\n").unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.message.contains("duplicate key \"a\""), "{}", e.message);
        assert!(e.message.contains("line 1"));
    }

    #[test]
    fn syntax_error_has_line() {
        let e = parse("a: [1, 2\nb: 3\n").unwrap_err();
        assert!(e.line >= 1);
    }

    #[test]
    fn aliases_rejected() {
        assert!(parse("a: &x 1\nb: *x\n").is_err());
    }

    #[test]
    fn multiple_documents_rejected() {
        assert!(parse("a: 1\n---\nb: 2\n").is_err());
    }

    #[test]
    fn flow_detection_and_spans() {
        let src = "a: [x, y]\nb:\n  - z\n";
        let n = parse(src).unwrap();
        match &n.get("a").unwrap().kind {
            NodeKind::Seq { flow, .. } => assert!(flow),
            _ => panic!(),
        }
        match &n.get("b").unwrap().kind {
            NodeKind::Seq { flow, .. } => assert!(!flow),
            _ => panic!(),
        }
        let a = n.get("a").unwrap();
        let text: String = src
            .chars()
            .skip(a.span.start.index)
            .take(a.span.end.index - a.span.start.index)
            .collect();
        assert_eq!(text, "[x, y]");
    }

    #[test]
    fn scalar_helpers() {
        let n = parse("a: true\nb: \"true\"\nc: 1.5\nd: 42\n").unwrap();
        assert_eq!(n.get("a").unwrap().as_bool(), Some(true));
        assert_eq!(n.get("b").unwrap().as_bool(), None);
        assert_eq!(n.get("c").unwrap().as_f64(), Some(1.5));
        assert_eq!(n.get("d").unwrap().as_i64(), Some(42));
    }
}
