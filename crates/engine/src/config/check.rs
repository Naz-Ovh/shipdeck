//! The first look at a deploy file: its shape against the JSON schema, so a
//! wrong key or a value of the wrong kind is caught with its line, all at
//! once. What passes is handed on as plain JSON for serde.

use serde_json::{Map, Value as Json};

use super::problem::{Problem, Span};
use super::types::Runner;
use super::yaml::{Node, Value};

/// What the shape check found.
pub struct Checked {
    /// The file as JSON, without the parts that did not fit.
    pub value: Json,
    pub problems: Vec<Problem>,
}

/// Checks a YAML tree against a config schema.
pub(super) fn structure_with(root: &Node, schema: &Json) -> Checked {
    let mut walk = Walk {
        root: schema,
        problems: Vec::new(),
    };
    let value = walk.node(root, schema, &mut Vec::new(), None);
    Checked {
        value: value.unwrap_or(Json::Null),
        problems: walk.problems,
    }
}

struct Walk<'s> {
    root: &'s Json,
    problems: Vec<Problem>,
}

/// The YAML kinds a schema accepts, for "must be …" messages.
fn wanted(types: &[&str]) -> String {
    let names: Vec<&str> = types
        .iter()
        .filter(|t| **t != "null")
        .map(|t| match *t {
            "string" => "text",
            "integer" => "a whole number",
            "number" => "a number",
            "boolean" => "true or false",
            "array" => "a list",
            "object" => "a map",
            other => other,
        })
        .collect();
    match names.as_slice() {
        [] => "nothing".into(),
        [one] => (*one).into(),
        [first @ .., last] => format!("{} or {last}", first.join(", ")),
    }
}

impl<'s> Walk<'s> {
    /// Follows `$ref` and a lone `allOf`, which is how schemars wraps a
    /// reference that has its own description.
    fn resolve(&self, schema: &'s Json) -> &'s Json {
        let mut schema = schema;
        loop {
            if let Some(reference) = schema.get("$ref").and_then(Json::as_str) {
                let name = reference.rsplit('/').next().unwrap_or_default();
                match self.root.pointer(&format!("/definitions/{name}")) {
                    Some(found) => schema = found,
                    None => return schema,
                }
            } else if let Some([only]) = schema
                .get("allOf")
                .and_then(Json::as_array)
                .map(Vec::as_slice)
            {
                schema = only;
            } else {
                return schema;
            }
        }
    }

    fn types(schema: &Json) -> Vec<&str> {
        match schema.get("type") {
            Some(Json::String(one)) => vec![one.as_str()],
            Some(Json::Array(many)) => many.iter().filter_map(Json::as_str).collect(),
            _ => Vec::new(),
        }
    }

    /// Whether `node` fits `schema` without any problem.
    fn fits(&self, node: &Node, schema: &'s Json) -> bool {
        let mut probe = Walk {
            root: self.root,
            problems: Vec::new(),
        };
        probe.node(node, schema, &mut Vec::new(), None);
        probe.problems.is_empty()
    }

    fn is_command(schema: &Json) -> bool {
        schema.get("title").and_then(Json::as_str) == Some("Command")
    }

    /// Checks one node; `key` is where it was named, for problems about the
    /// node as a whole.
    fn node(
        &mut self,
        node: &Node,
        schema: &'s Json,
        path: &mut Vec<String>,
        key: Option<Span>,
    ) -> Option<Json> {
        let schema = self.resolve(schema);
        let at = Some(if node.is_null() {
            key.unwrap_or(node.span)
        } else {
            node.span
        });

        if Self::is_command(schema) {
            return match &node.value {
                Value::Map(_) => self.command_map(node, path, at),
                Value::Scalar {
                    text, tag: None, ..
                } if !node.is_null() => Some(Json::String(text.clone())),
                _ => {
                    self.mismatch(node, schema, &["string", "object"], path, at);
                    None
                }
            };
        }

        if let Some(choices) = schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Json::as_array)
        {
            return self.union(node, choices, path, key, at);
        }
        if let Some(values) = schema.get("enum").and_then(Json::as_array) {
            return self.enum_value(node, values, path, at);
        }
        if let Some(constant) = schema.get("const").and_then(Json::as_str) {
            return self.constant(node, constant, path, at);
        }

        let types = Self::types(schema);
        if types.is_empty() {
            return Some(plain_json(node));
        }
        if node.is_null() && types.contains(&"null") {
            return Some(Json::Null);
        }
        match &node.value {
            Value::Map(_) if types.contains(&"object") => {
                Some(self.object(node, schema, path, key))
            }
            Value::Seq(items) if types.contains(&"array") => Some(self.array(items, schema, path)),
            Value::Scalar { .. } if !node.is_null() => self.scalar(node, schema, &types, path, at),
            _ => {
                if node.is_null() {
                    self.problems.push(
                        Problem::new(format!("`{}` is empty", dotted(path)))
                            .at(at)
                            .label(format!("needs {}", wanted(&types)))
                            .help("give it a value, or remove the line to use the default"),
                    );
                } else {
                    self.mismatch(node, schema, &types, path, at);
                }
                None
            }
        }
    }

    /// A node where `anyOf` or `oneOf` gives the shapes it may take.
    fn union(
        &mut self,
        node: &Node,
        choices: &'s [Json],
        path: &mut Vec<String>,
        key: Option<Span>,
        at: Option<Span>,
    ) -> Option<Json> {
        // A command map is checked in one place, so its own message is the
        // only one the reader sees.
        if matches!(node.value, Value::Map(_))
            && let Some(command) = choices
                .iter()
                .find(|choice| Self::is_command(self.resolve(choice)))
        {
            return self.node(node, command, path, key);
        }
        if let Some(fit) = choices.iter().find(|choice| self.fits(node, choice)) {
            return self.node(node, fit, path, key);
        }
        let kind = kind_of(node);
        if let Some(same) = choices.iter().find(|choice| {
            let choice = self.resolve(choice);
            Self::types(choice).contains(&kind) && choice.get("const").is_none()
        }) {
            return self.node(node, same, path, key);
        }
        let consts: Vec<&str> = choices
            .iter()
            .filter_map(|c| self.resolve(c).get("const").and_then(Json::as_str))
            .collect();
        if consts.is_empty() {
            let kinds: Vec<&str> = choices
                .iter()
                .flat_map(|c| Self::types(self.resolve(c)))
                .collect();
            self.problems.push(
                Problem::new(format!("`{}` must be {}", dotted(path), wanted(&kinds)))
                    .at(at)
                    .label(format!("found {}", node.kind())),
            );
        } else {
            self.problems.push(
                Problem::new(format!("`{}` must be {}", dotted(path), or_list(&consts)))
                    .at(at)
                    .label(format!("found {}", node.kind()))
                    .help(choice_help(self, choices)),
            );
        }
        None
    }

    /// A node where `enum` gives the values it may take.
    fn enum_value(
        &mut self,
        node: &Node,
        values: &'s [Json],
        path: &[String],
        at: Option<Span>,
    ) -> Option<Json> {
        let allowed: Vec<&str> = values.iter().filter_map(Json::as_str).collect();
        if let Value::Scalar { text, .. } = &node.value
            && allowed.contains(&text.as_str())
        {
            return Some(Json::String(text.clone()));
        }
        self.problems.push(
            Problem::new(format!("`{}` must be {}", dotted(path), or_list(&allowed)))
                .at(at)
                .label(format!("found {}", node.kind())),
        );
        None
    }

    /// A node where `const` fixes the value.
    fn constant(
        &mut self,
        node: &Node,
        constant: &str,
        path: &[String],
        at: Option<Span>,
    ) -> Option<Json> {
        if matches!(&node.value, Value::Scalar { text, .. } if text == constant) {
            return Some(Json::String(constant.to_string()));
        }
        self.problems.push(
            Problem::new(format!("`{}` must be {constant}", dotted(path)))
                .at(at)
                .label(format!("found {}", node.kind())),
        );
        None
    }

    /// A command map: exactly one known runner and its code.
    fn command_map(&mut self, node: &Node, path: &[String], at: Option<Span>) -> Option<Json> {
        let Value::Map(entries) = &node.value else {
            return None;
        };
        let help = "a command is text, or one runner and its code. Put the whole command in \
                    quotes if it holds `: `";
        match entries.as_slice() {
            [(key, value)] if Runner::from_key(&key.text).is_some() => {
                let Value::Scalar {
                    text, tag: None, ..
                } = &value.value
                else {
                    self.problems.push(
                        Problem::new(format!("`{}` runner code must be text", dotted(path)))
                            .at(Some(value.span))
                            .label(format!("found {}", value.kind())),
                    );
                    return None;
                };
                let mut map = Map::new();
                map.insert(key.text.clone(), Json::String(text.clone()));
                Some(Json::Object(map))
            }
            _ => {
                let keys: Vec<String> = entries.iter().map(|(key, _)| key.text.clone()).collect();
                self.problems.push(
                    Problem::new(format!(
                        "`{}` reads as a command map with {}",
                        dotted(path),
                        keys.join(", ")
                    ))
                    .at(at)
                    .label("not a command")
                    .help(help),
                );
                None
            }
        }
    }

    /// Each item of an array, checked against `items`.
    fn array(&mut self, items: &[Node], schema: &'s Json, path: &mut Vec<String>) -> Json {
        let items_schema = schema.get("items").unwrap_or(&Json::Bool(true));
        let mut out = Vec::new();
        for (index, item) in items.iter().enumerate() {
            path.push(format!("[{index}]"));
            if let Some(value) = self.node(item, items_schema, path, Some(item.span)) {
                out.push(value);
            }
            path.pop();
        }
        Json::Array(out)
    }

    /// A scalar checked against `types`.
    fn scalar(
        &mut self,
        node: &Node,
        schema: &'s Json,
        types: &[&str],
        path: &[String],
        at: Option<Span>,
    ) -> Option<Json> {
        let Value::Scalar { text, plain, tag } = &node.value else {
            return None;
        };
        if let Some(tag) = tag {
            let written = format!("{tag} {text}").trim().to_string();
            self.problems.push(
                Problem::new(format!("`{}` reads as a YAML tag", dotted(path)))
                    .at(at)
                    .label("YAML treats a leading ! as a tag")
                    .help(format!("put it in quotes: \"{written}\"")),
            );
            return None;
        }
        if types.contains(&"string") {
            return Some(Json::String(text.clone()));
        }
        if types.contains(&"integer")
            && *plain
            && let Ok(number) = text.parse::<u64>()
        {
            if let Some(max) = schema.get("maximum").and_then(Json::as_u64)
                && number > max
            {
                self.problems.push(
                    Problem::new(format!("`{}` must be at most {max}", dotted(path)))
                        .at(at)
                        .label("too large"),
                );
                return None;
            }
            return Some(Json::from(number));
        }
        if types.contains(&"boolean") && *plain && matches!(text.as_str(), "true" | "false") {
            return Some(Json::Bool(text == "true"));
        }
        self.mismatch(node, schema, types, path, at);
        None
    }

    fn mismatch(
        &mut self,
        node: &Node,
        schema: &Json,
        types: &[&str],
        path: &[String],
        at: Option<Span>,
    ) {
        let mut problem = Problem::new(format!("`{}` must be {}", dotted(path), wanted(types)))
            .at(at)
            .label(format!("found {}", node.kind()));
        if let Some(properties) = schema.get("properties").and_then(Json::as_object) {
            let sides: Vec<&str> = ["local", "server"]
                .into_iter()
                .filter(|side| properties.contains_key(*side))
                .collect();
            if !sides.is_empty() && matches!(node.value, Value::Scalar { .. } | Value::Seq(_)) {
                let example = match &node.value {
                    Value::Scalar { text, .. } => text.clone(),
                    _ => "…".into(),
                };
                let last = path.last().map_or("step", String::as_str);
                problem = problem
                    .label("say where it runs")
                    .help(format!("`{last}: {{ {}: {example} }}`", sides[0]));
            }
        }
        self.problems.push(problem);
    }

    fn object(
        &mut self,
        node: &Node,
        schema: &'s Json,
        path: &mut Vec<String>,
        key: Option<Span>,
    ) -> Json {
        let properties = schema.get("properties").and_then(Json::as_object);
        let extra = schema.get("additionalProperties");
        let mut out = Map::new();
        for (key, value) in node.entries() {
            path.push(key.text.clone());
            let property = properties.and_then(|p| p.get(&key.text));
            let schema_for = match (property, extra) {
                (Some(property), _) => Some(property),
                (None, Some(Json::Bool(false))) => None,
                (None, Some(other)) => Some(other),
                (None, None) => Some(&Json::Bool(true)),
            };
            if let Some(child) = schema_for {
                // A key with nothing after it means "the default".
                if value.is_null() && property.is_some() && !required(schema, &key.text) {
                    path.pop();
                    continue;
                }
                if let Some(json) = self.node(value, child, path, Some(key.span)) {
                    out.insert(key.text.clone(), json);
                }
            } else {
                let known: Vec<&str> = properties
                    .map(|p| p.keys().map(String::as_str).collect())
                    .unwrap_or_default();
                self.problems.push(unknown_key(path, &known, key.span));
            }
            path.pop();
        }
        if let Some(required) = schema.get("required").and_then(Json::as_array) {
            for name in required.iter().filter_map(Json::as_str) {
                if !node.entries().iter().any(|(k, _)| k.text == name) {
                    path.push(name.to_string());
                    let at = if path.len() == 1 { None } else { key };
                    self.problems.push(
                        Problem::new(format!("`{}` is missing", dotted(path)))
                            .at(at)
                            .label(format!("needs `{name}`"))
                            .help(missing_help(path)),
                    );
                    path.pop();
                }
            }
        }
        Json::Object(out)
    }
}

fn required(schema: &Json, name: &str) -> bool {
    schema
        .get("required")
        .and_then(Json::as_array)
        .is_some_and(|list| list.iter().any(|r| r.as_str() == Some(name)))
}

fn kind_of(node: &Node) -> &'static str {
    match &node.value {
        _ if node.is_null() => "null",
        Value::Scalar { .. } => "string",
        Value::Seq(_) => "array",
        Value::Map(_) => "object",
    }
}

/// Plain YAML as JSON, for parts the schema leaves open. An unquoted
/// scalar is typed by the YAML 1.2 core schema, so `instances: 2` is a
/// number and `"2"` stays text.
fn plain_json(node: &Node) -> Json {
    match &node.value {
        _ if node.is_null() => Json::Null,
        Value::Scalar {
            text,
            plain: true,
            tag: None,
        } => core_schema(text).unwrap_or_else(|| Json::String(text.clone())),
        Value::Scalar { text, .. } => Json::String(text.clone()),
        Value::Seq(items) => Json::Array(items.iter().map(plain_json).collect()),
        Value::Map(entries) => Json::Object(
            entries
                .iter()
                .map(|(k, v)| (k.text.clone(), plain_json(v)))
                .collect(),
        ),
    }
}

/// A plain scalar as the YAML 1.2 core schema reads it (spec 10.3.2):
/// booleans, integers in decimal, `0o` octal or `0x` hex, and floats.
/// `.inf` and `.nan` have no JSON form, so they stay text, as does
/// everything else.
fn core_schema(text: &str) -> Option<Json> {
    match text {
        "true" | "True" | "TRUE" => return Some(Json::Bool(true)),
        "false" | "False" | "FALSE" => return Some(Json::Bool(false)),
        _ => {}
    }
    let digits = |s: &str, radix: u32| !s.is_empty() && s.chars().all(|c| c.is_digit(radix));
    if let Some(octal) = text.strip_prefix("0o").filter(|s| digits(s, 8)) {
        return i64::from_str_radix(octal, 8).ok().map(Json::from);
    }
    if let Some(hex) = text.strip_prefix("0x").filter(|s| digits(s, 16)) {
        return i64::from_str_radix(hex, 16).ok().map(Json::from);
    }
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    if digits(unsigned, 10) {
        return text.parse::<i64>().ok().map(Json::from);
    }
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e.strip_prefix(['-', '+']).unwrap_or(e))),
        None => (unsigned, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some((whole, fraction)) => {
            (whole.is_empty() && digits(fraction, 10))
                || (digits(whole, 10) && (fraction.is_empty() || digits(fraction, 10)))
        }
        None => digits(mantissa, 10),
    };
    if mantissa_ok && exponent.is_none_or(|e| digits(e, 10)) {
        return text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Json::Number);
    }
    None
}

fn dotted(path: &[String]) -> String {
    let mut out = String::new();
    for part in path {
        if !part.starts_with('[') && !out.is_empty() {
            out.push('.');
        }
        out.push_str(part);
    }
    out
}

fn or_list(items: &[&str]) -> String {
    match items {
        [] => "nothing".into(),
        [one] => (*one).into(),
        [first @ .., last] => format!("{} or {last}", first.join(", ")),
    }
}

fn choice_help(walk: &Walk, choices: &[Json]) -> String {
    choices
        .iter()
        .map(|c| walk.resolve(c))
        .filter_map(|c| {
            let name = c.get("const").and_then(Json::as_str)?;
            let about = c
                .get("description")
                .and_then(Json::as_str)
                .map(|d| d.split(". ").next().unwrap_or(d).trim_end_matches('.'))
                .unwrap_or_default();
            let mut chars = about.chars();
            let about = match chars.next() {
                Some(first) => first.to_lowercase().chain(chars).collect(),
                None => String::new(),
            };
            Some(format!("`{name}` ({about})"))
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

fn unknown_key(path: &[String], known: &[&str], at: Span) -> Problem {
    let key = path.last().map(String::as_str).unwrap_or_default();
    let parent = dotted(&path[..path.len() - 1]);
    let full = dotted(path);
    let mut problem = Problem::new(format!("unknown key `{full}`"))
        .at(Some(at))
        .label("unknown key");
    let close = known
        .iter()
        .filter(|name| similar(name, key))
        .copied()
        .collect::<Vec<_>>();
    problem = match close.as_slice() {
        [one] => problem.help(format!("did you mean `{one}`?")),
        _ if known.is_empty() => problem,
        _ if parent.is_empty() => {
            problem.help(format!("the top level takes {}", known_list(known)))
        }
        _ => problem.help(format!("`{parent}` takes {}", known_list(known))),
    };
    problem
}

fn known_list(known: &[&str]) -> String {
    or_and(known, "and")
}

fn or_and(items: &[&str], word: &str) -> String {
    match items {
        [] => "nothing".into(),
        [one] => (*one).into(),
        [first @ .., last] => format!("{} {word} {last}", first.join(", ")),
    }
}

/// Close enough to be a typo: one or two edits apart, or one contains the
/// other.
fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    if a == b || (b.len() >= 3 && (a.contains(&b) || b.contains(&a))) {
        return true;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            prev = cur;
        }
    }
    row[b.len()] <= 2.min(a.len().max(b.len()) / 3 + 1)
}

fn missing_help(path: &[String]) -> String {
    match dotted(path).as_str() {
        "name" => "add `name: shop` at the top".into(),
        "server" => "add `server: { ssh: { host: my-server }, dir: /opt/shop }`".into(),
        "server.ssh" => "add `ssh: { host: my-server }`".into(),
        "server.ssh.host" => {
            "add `host:` with the server's address or a ~/.ssh/config alias".into()
        }
        "server.dir" => "add `dir: /opt/shop`, the folder that holds current and releases".into(),
        "apps" => "add at least one app under `apps:`".into(),
        other => format!("add `{}:`", other.rsplit('.').next().unwrap_or(other)),
    }
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;
