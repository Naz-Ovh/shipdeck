//! A YAML document as a small tree that remembers where each part was
//! written, so a problem can point at its line and column.

use std::collections::HashMap;

use granit_parser::{Event, Parser, ScalarStyle, ScanError};

use super::problem::{Problem, Span};

#[derive(Debug, Clone)]
pub struct Node {
    pub value: Value,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Value {
    Scalar {
        text: String,
        /// Unquoted, so `true`, `12` and `~` keep their YAML meaning.
        plain: bool,
        /// A local tag such as `!src/old`, which is usually a `!pattern`
        /// that needed quotes.
        tag: Option<String>,
    },
    Seq(Vec<Node>),
    Map(Vec<(Key, Node)>),
}

#[derive(Debug, Clone)]
pub struct Key {
    pub text: String,
    pub span: Span,
}

impl Node {
    /// `~`, `null` or nothing at all, as in `key:` with no value.
    pub fn is_null(&self) -> bool {
        matches!(&self.value, Value::Scalar { text, plain: true, tag: None }
            if matches!(text.as_str(), "" | "~" | "null" | "Null" | "NULL"))
    }

    /// What the node is, for "found …" labels.
    pub fn kind(&self) -> String {
        match &self.value {
            _ if self.is_null() => "nothing".into(),
            Value::Scalar { text, .. } => {
                let text = text.lines().next().unwrap_or_default();
                if text.chars().count() > 30 {
                    format!("`{}…`", text.chars().take(29).collect::<String>())
                } else {
                    format!("`{text}`")
                }
            }
            Value::Seq(_) => "a list".into(),
            Value::Map(_) => "a map".into(),
        }
    }

    pub fn entries(&self) -> &[(Key, Node)] {
        match &self.value {
            Value::Map(entries) => entries,
            _ => &[],
        }
    }

    /// The node at `path` below this one.
    pub fn find(&self, path: &[&str]) -> Option<&Node> {
        let Some((first, rest)) = path.split_first() else {
            return Some(self);
        };
        let node = match &self.value {
            Value::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.text == *first)
                .map(|(_, v)| v),
            Value::Seq(items) => first.parse::<usize>().ok().and_then(|i| items.get(i)),
            Value::Scalar { .. } => None,
        }?;
        node.find(rest)
    }

    /// Where the value at `path` was written. A map or list points at its
    /// key instead, which reads better than its first child.
    pub fn span_of(&self, path: &[&str]) -> Option<Span> {
        let node = self.find(path)?;
        match node.value {
            Value::Scalar { .. } if !node.is_null() => Some(node.span),
            _ => self.key_span(path),
        }
    }

    /// Where the key at the end of `path` was written.
    pub fn key_span(&self, path: &[&str]) -> Option<Span> {
        let (last, parent) = path.split_last()?;
        let parent = self.find(parent)?;
        match &parent.value {
            Value::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.text == *last)
                .map(|(k, _)| k.span),
            Value::Seq(items) => last
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get(i))
                .map(|n| n.span),
            Value::Scalar { .. } => None,
        }
    }
}

/// Reads one YAML document. An empty file is an empty map. Lines end in LF
/// only: a CR anywhere is refused.
pub fn parse(text: &str) -> Result<Node, Problem> {
    if let Some(at) = text.find('\r') {
        let before = &text[..at];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().unwrap_or(before).chars().count();
        return Err(
            Problem::new("deploy.yaml has Windows line endings (CRLF); shipdeck reads LF only")
                .at(Some(Span::point(line, col)))
                .label("a CR here")
                .help("save it with LF line endings; in Git, `*.yaml text eol=lf` in .gitattributes keeps them"),
        );
    }
    let mut reader = Reader {
        parser: Parser::new_from_str(text),
        anchors: HashMap::new(),
    };
    let mut root = None;
    loop {
        let (event, span) = reader.next()?;
        match event {
            Event::StreamStart | Event::DocumentEnd => {}
            Event::StreamEnd => break,
            Event::DocumentStart(..) => {
                if root.is_some() {
                    return Err(Problem::new("deploy.yaml holds more than one document")
                        .at(Some(span))
                        .label("a second document starts here")
                        .help("remove the `---` line and what follows it, or merge them"));
                }
                let (event, span) = reader.next()?;
                if matches!(event, Event::DocumentEnd) {
                    continue;
                }
                root = Some(reader.node(event, span)?);
            }
            _ => {}
        }
    }
    Ok(root.unwrap_or(Node {
        value: Value::Map(Vec::new()),
        span: Span::point(1, 0),
    }))
}

struct Reader<'a> {
    parser: Parser<'a, granit_parser::StrInput<'a>>,
    anchors: HashMap<usize, Node>,
}

impl<'a> Reader<'a> {
    fn next(&mut self) -> Result<(Event<'a>, Span), Problem> {
        match self.parser.next_event() {
            Some(Ok((event, range))) => {
                // A tag written before the value is where the eye goes.
                let start = range.tag_start.unwrap_or(range.start);
                Ok((
                    event,
                    Span {
                        start: (start.line(), start.col()),
                        end: (range.end.line(), range.end.col()),
                    },
                ))
            }
            Some(Err(err)) => Err(syntax(&err)),
            None => Err(Problem::new("deploy.yaml ends too early")),
        }
    }

    fn node(&mut self, event: Event<'a>, span: Span) -> Result<Node, Problem> {
        let (node, anchor) = match event {
            Event::Scalar(text, style, anchor, tag) => (
                Node {
                    value: Value::Scalar {
                        text: text.into_owned(),
                        plain: style == ScalarStyle::Plain,
                        tag: tag
                            .map(|tag| tag.to_string())
                            .filter(|tag| tag.starts_with('!') && !tag.starts_with("!!")),
                    },
                    span,
                },
                anchor,
            ),
            Event::Alias(id) => {
                let Some(found) = self.anchors.get(&id) else {
                    return Err(Problem::new("this alias names no anchor").at(Some(span)));
                };
                return Ok(Node {
                    value: found.value.clone(),
                    span,
                });
            }
            Event::SequenceStart(_, anchor, _) => {
                let mut items = Vec::new();
                loop {
                    let (event, at) = self.next()?;
                    if matches!(event, Event::SequenceEnd) {
                        break;
                    }
                    items.push(self.node(event, at)?);
                }
                (
                    Node {
                        value: Value::Seq(items),
                        span,
                    },
                    anchor,
                )
            }
            Event::MappingStart(_, anchor, _) => {
                let mut entries: Vec<(Key, Node)> = Vec::new();
                loop {
                    let (event, at) = self.next()?;
                    if matches!(event, Event::MappingEnd) {
                        break;
                    }
                    let key = self.node(event, at)?;
                    let Value::Scalar { text, .. } = key.value else {
                        return Err(Problem::new("a key must be plain text")
                            .at(Some(key.span))
                            .label("not text"));
                    };
                    if let Some((first, _)) = entries.iter().find(|(k, _)| k.text == text) {
                        return Err(Problem::new(format!("`{text}` is written twice"))
                            .at(Some(key.span))
                            .label(format!("first written on line {}", first.span.start.0))
                            .help("keep one of them"));
                    }
                    let (event, at) = self.next()?;
                    let value = self.node(event, at)?;
                    entries.push((
                        Key {
                            text,
                            span: key.span,
                        },
                        value,
                    ));
                }
                (
                    Node {
                        value: Value::Map(entries),
                        span,
                    },
                    anchor,
                )
            }
            other => {
                return Err(Problem::new(format!("unexpected YAML: {other:?}")).at(Some(span)));
            }
        };
        if anchor != 0 {
            self.anchors.insert(anchor, node.clone());
        }
        Ok(node)
    }
}

fn syntax(err: &ScanError) -> Problem {
    let marker = err.marker();
    let text = err.to_string();
    // Display adds " at char N line L column C"; the snippet says that already.
    let message = text.split(" at char ").next().unwrap_or(&text).to_string();
    Problem::new(format!("this is not valid YAML: {message}"))
        .at(Some(Span::point(marker.line(), marker.col())))
        .label("here")
}
