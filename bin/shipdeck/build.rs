//! Turns the manual, `manual/index.yaml`, `manual/details.yaml` and
//! `manual/configs.yaml`, into static Rust data in `$OUT_DIR/manual.rs`, so
//! `shipdeck man` reads no file and parses nothing at run time. A mistake in
//! any of them fails the build, with its file and line.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use granit_parser::{Event, Parser, StrInput};

const INDEX: &str = "manual/index.yaml";
const DETAILS: &str = "manual/details.yaml";
const CONFIGS: &str = "manual/configs.yaml";
/// So a page reads well in a terminal and in a model's context alike.
const MAX_WIDTH: usize = 100;

fn main() {
    for file in [INDEX, DETAILS, CONFIGS] {
        println!("cargo::rerun-if-changed={file}");
    }
    if let Err(err) = generate() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

/// A section of the index, with its topics in order.
struct Section {
    name: String,
    topics: Vec<Topic>,
}

struct Topic {
    key: String,
    brief: String,
    line: usize,
    see: Vec<String>,
    text: String,
}

/// One deploy.yaml property, in the tree's order.
struct Config {
    key: String,
    depth: usize,
    kind: String,
    brief: String,
    see: Vec<String>,
    text: String,
}

fn generate() -> Result<(), String> {
    let mut sections = index(&read(INDEX)?)?;
    details(&read(DETAILS)?, &mut sections)?;
    let topics: HashSet<String> = sections
        .iter()
        .flat_map(|s| &s.topics)
        .map(|t| t.key.clone())
        .collect();
    let mut configs = Vec::new();
    tree(&read(CONFIGS)?, "", 0, &topics, &mut configs)?;
    let out = PathBuf::from(std::env::var("OUT_DIR").map_err(|err| format!("OUT_DIR: {err}"))?)
        .join("manual.rs");
    std::fs::write(&out, rust(&sections, &configs))
        .map_err(|err| format!("{}: {err}", out.display()))
}

/// `index.yaml`: a list of sections, each `{ section, topics }`, each topic
/// `{ key, brief }`.
fn index(root: &Node) -> Result<Vec<Section>, String> {
    let mut sections = Vec::new();
    let mut keys = HashSet::new();
    for node in root.list(INDEX)? {
        let fields = node.map(INDEX, &["section", "topics"], &[])?;
        let mut section = Section {
            name: one_line(INDEX, fields[0].unwrap())?,
            topics: Vec::new(),
        };
        for node in fields[1].unwrap().list(INDEX)? {
            let fields = node.map(INDEX, &["key", "brief"], &[])?;
            let key = one_line(INDEX, fields[0].unwrap())?;
            if !is_key(&key) {
                return Err(at(
                    INDEX,
                    node.line,
                    &format!(
                        "{key:?} is not a key: lowercase letters and digits, joined by `.` or `-`"
                    ),
                ));
            }
            if !keys.insert(key.clone()) {
                return Err(at(INDEX, node.line, &format!("`{key}` is listed twice")));
            }
            section.topics.push(Topic {
                key,
                brief: one_line(INDEX, fields[1].unwrap())?,
                line: node.line,
                see: Vec::new(),
                text: String::new(),
            });
        }
        sections.push(section);
    }
    Ok(sections)
}

/// `details.yaml`: a map from each index key to `{ see, text }`.
fn details(root: &Node, sections: &mut [Section]) -> Result<(), String> {
    let Value::Map(entries) = &root.value else {
        return Err(at(
            DETAILS,
            root.line,
            "must be a map from each key to its topic",
        ));
    };
    let known: HashSet<String> = sections
        .iter()
        .flat_map(|s| &s.topics)
        .map(|t| t.key.clone())
        .collect();
    for (key, node) in entries {
        let Some(topic) = sections
            .iter_mut()
            .flat_map(|s| &mut s.topics)
            .find(|t| t.key == *key)
        else {
            return Err(at(
                DETAILS,
                node.line,
                &format!("`{key}` is not in {INDEX}"),
            ));
        };
        let fields = node.map(DETAILS, &["text"], &["see"])?;
        topic.text = text(DETAILS, fields[0].unwrap())?;
        topic.see = see(DETAILS, fields[1], key, &known)?;
    }
    for topic in sections.iter().flat_map(|s| &s.topics) {
        if topic.text.is_empty() {
            return Err(at(
                INDEX,
                topic.line,
                &format!("`{}` has no entry in {DETAILS}", topic.key),
            ));
        }
    }
    Ok(())
}

/// `configs.yaml`: a map from each property to `{ type, brief, text, see }`,
/// a block's properties under `fields`. Paths join with `.`.
fn tree(
    node: &Node,
    parent: &str,
    depth: usize,
    topics: &HashSet<String>,
    out: &mut Vec<Config>,
) -> Result<(), String> {
    let Value::Map(entries) = &node.value else {
        return Err(at(CONFIGS, node.line, "must be a map of properties"));
    };
    for (name, node) in entries {
        let key = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}.{name}")
        };
        if topics.contains(&key) {
            return Err(at(
                CONFIGS,
                node.line,
                &format!("`{key}` is also a topic key; `man details` could not tell them apart"),
            ));
        }
        let fields = node.map(CONFIGS, &["brief"], &["type", "text", "see", "fields"])?;
        let &[brief, kind, body, related, children] = fields.as_slice() else {
            unreachable!("map returns one entry per key");
        };
        if kind.is_some() == children.is_some() {
            return Err(at(
                CONFIGS,
                node.line,
                &format!("`{key}` needs either `type` (a value) or `fields` (a block)"),
            ));
        }
        out.push(Config {
            key: key.clone(),
            depth,
            kind: kind
                .map(|k| one_line(CONFIGS, k))
                .transpose()?
                .unwrap_or_default(),
            brief: one_line(CONFIGS, brief.unwrap())?,
            see: see(CONFIGS, related, &key, topics)?,
            text: body
                .map(|b| text(CONFIGS, b))
                .transpose()?
                .unwrap_or_default(),
        });
        if let Some(children) = children {
            tree(children, &key, depth + 1, topics, out)?;
        }
    }
    Ok(())
}

/// A page of text: not empty, lines at most [`MAX_WIDTH`], no tabs.
fn text(file: &str, node: &Node) -> Result<String, String> {
    let Value::Text(text) = &node.value else {
        return Err(at(file, node.line, "`text` must be text"));
    };
    if text.trim().is_empty() {
        return Err(at(file, node.line, "`text` is empty"));
    }
    for (offset, line) in text.lines().enumerate() {
        if line.chars().count() > MAX_WIDTH || line.contains('\t') {
            return Err(at(
                file,
                node.line + 1 + offset,
                &format!("lines are at most {MAX_WIDTH} characters, without tabs"),
            ));
        }
    }
    Ok(text.trim_end().to_string())
}

/// Related topic keys, each one that exists and is not `key` itself.
fn see(
    file: &str,
    node: Option<&Node>,
    key: &str,
    topics: &HashSet<String>,
) -> Result<Vec<String>, String> {
    let Some(node) = node else {
        return Ok(Vec::new());
    };
    node.list(file)?
        .iter()
        .map(|item| {
            let see = one_line(file, item)?;
            if see == key || !topics.contains(&see) {
                return Err(at(
                    file,
                    item.line,
                    &format!("`{key}` sees `{see}`, which is not another topic"),
                ));
            }
            Ok(see)
        })
        .collect()
}

/// The data as Rust: `{:?}` writes each text as a Rust string literal.
fn rust(sections: &[Section], configs: &[Config]) -> String {
    let mut out = format!("// Generated by build.rs from {INDEX}, {DETAILS} and {CONFIGS}.\n\n");
    out.push_str("pub static SECTIONS: &[Section] = &[\n");
    for section in sections {
        let _ = writeln!(
            out,
            "    Section {{\n        name: {:?},\n        topics: &[",
            section.name
        );
        for t in &section.topics {
            let _ = writeln!(
                out,
                "            Topic {{ key: {:?}, brief: {:?}, see: &{:?}, text: {:?} }},",
                t.key, t.brief, t.see, t.text
            );
        }
        out.push_str("        ],\n    },\n");
    }
    out.push_str("];\n\npub static CONFIGS: &[Config] = &[\n");
    for c in configs {
        let _ = writeln!(
            out,
            "    Config {{ key: {:?}, depth: {}, kind: {:?}, brief: {:?}, see: &{:?}, text: {:?} }},",
            c.key, c.depth, c.kind, c.brief, c.see, c.text
        );
    }
    out.push_str("];\n");
    out
}

/// `config`, `step.preflight`, `exit-codes`.
fn is_key(key: &str) -> bool {
    key.split(['.', '-']).all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

/// One line of text. No `"`, since briefs and types are printed inside
/// `"..."` attributes.
fn one_line(file: &str, node: &Node) -> Result<String, String> {
    match &node.value {
        Value::Text(text) if text.contains('"') => {
            Err(at(file, node.line, "must not hold `\"`; use backticks"))
        }
        Value::Text(text) if !text.trim().is_empty() && !text.contains('\n') => {
            Ok(text.trim().to_string())
        }
        _ => Err(at(file, node.line, "must be one line of text")),
    }
}

fn at(file: &str, line: usize, message: &str) -> String {
    format!("{file}:{line}: {message}")
}

/// A YAML node with the line it starts on.
struct Node {
    line: usize,
    value: Value,
}

enum Value {
    Text(String),
    List(Vec<Node>),
    Map(Vec<(String, Node)>),
}

impl Node {
    fn list(&self, file: &str) -> Result<&[Node], String> {
        match &self.value {
            Value::List(items) => Ok(items),
            _ => Err(at(file, self.line, "must be a list")),
        }
    }

    /// The values of `required` then `optional`, in that order; any other
    /// key is an error.
    fn map(
        &self,
        file: &str,
        required: &[&str],
        optional: &[&str],
    ) -> Result<Vec<Option<&Node>>, String> {
        let all: Vec<&str> = required.iter().chain(optional).copied().collect();
        let Value::Map(entries) = &self.value else {
            return Err(at(
                file,
                self.line,
                &format!("must be a map of {}", all.join(", ")),
            ));
        };
        if let Some((key, node)) = entries.iter().find(|(k, _)| !all.contains(&k.as_str())) {
            return Err(at(
                file,
                node.line,
                &format!("unknown key `{key}`; it takes {}", all.join(", ")),
            ));
        }
        let mut found = Vec::new();
        for key in &all {
            let node = entries.iter().find(|(k, _)| k == key).map(|(_, node)| node);
            if node.is_none() && required.contains(key) {
                return Err(at(file, self.line, &format!("`{key}` is missing")));
            }
            found.push(node);
        }
        Ok(found)
    }
}

fn read(file: &str) -> Result<Node, String> {
    let text = std::fs::read_to_string(file).map_err(|err| format!("{file}: {err}"))?;
    let mut reader = Reader {
        file,
        parser: Parser::new_from_str(&text),
    };
    loop {
        match reader.next()? {
            (Event::DocumentStart(..), _) => break,
            (Event::StreamEnd, line) => return Err(at(file, line, "the file is empty")),
            _ => {}
        }
    }
    let (event, line) = reader.next()?;
    reader.node(event, line)
}

/// Reads a YAML document's events into a [`Node`] tree. The manual needs
/// text, lists and maps only, so anchors, aliases and tags are refused.
struct Reader<'a> {
    file: &'a str,
    parser: Parser<'a, StrInput<'a>>,
}

impl<'a> Reader<'a> {
    fn next(&mut self) -> Result<(Event<'a>, usize), String> {
        match self.parser.next_event() {
            Some(Ok((event, span))) => Ok((event, span.start.line())),
            Some(Err(err)) => Err(format!("{}: {err}", self.file)),
            None => Err(format!("{}: ends too early", self.file)),
        }
    }

    fn node(&mut self, event: Event<'a>, line: usize) -> Result<Node, String> {
        let value = match event {
            Event::Scalar(text, _, 0, None) => Value::Text(text.into_owned()),
            Event::SequenceStart(_, 0, None) => {
                let mut items = Vec::new();
                loop {
                    match self.next()? {
                        (Event::SequenceEnd, _) => break,
                        (event, line) => items.push(self.node(event, line)?),
                    }
                }
                Value::List(items)
            }
            Event::MappingStart(_, 0, None) => {
                let mut entries: Vec<(String, Node)> = Vec::new();
                loop {
                    let (event, at_line) = match self.next()? {
                        (Event::MappingEnd, _) => break,
                        found => found,
                    };
                    let Event::Scalar(key, _, 0, None) = event else {
                        return Err(at(self.file, at_line, "a key must be plain text"));
                    };
                    let key = key.into_owned();
                    if entries.iter().any(|(k, _)| *k == key) {
                        return Err(at(self.file, at_line, &format!("`{key}` is written twice")));
                    }
                    let (event, line) = self.next()?;
                    entries.push((key, self.node(event, line)?));
                }
                Value::Map(entries)
            }
            _ => {
                return Err(at(
                    self.file,
                    line,
                    "the manual takes text, lists and maps; no anchors, aliases or tags",
                ));
            }
        };
        Ok(Node { line, value })
    }
}
