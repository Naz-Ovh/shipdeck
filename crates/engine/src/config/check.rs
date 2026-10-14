//! The first look at a deploy.yaml: its shape against the JSON schema, so
//! a wrong key or a value of the wrong kind is caught with its line, all at
//! once. What passes is handed on as plain JSON for serde.

use serde_json::{Map, Value as Json};

use super::problem::{Problem, Span};
use super::schema;
use super::yaml::{Node, Value};

/// What the shape check found.
pub struct Checked {
    /// The file as JSON, without the parts that did not fit.
    pub value: Json,
    pub problems: Vec<Problem>,
    /// It uses keys from shipdeck 0.2, so other problems are mostly echoes.
    pub old_format: bool,
}

/// Checks `root` against the schema.
pub fn structure(root: &Node) -> Checked {
    let schema = schema::generate();
    let mut walk = Walk {
        root: &schema,
        problems: Vec::new(),
        old_format: false,
    };
    let value = walk.node(root, &schema, &mut Vec::new(), None);
    Checked {
        value: value.unwrap_or(Json::Null),
        problems: walk.problems,
        old_format: walk.old_format,
    }
}

struct Walk<'s> {
    root: &'s Json,
    problems: Vec<Problem>,
    old_format: bool,
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
            old_format: false,
        };
        probe.node(node, schema, &mut Vec::new(), None);
        probe.problems.is_empty()
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

        if let Some(choices) = schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Json::as_array)
        {
            if let Some(fit) = choices.iter().find(|choice| self.fits(node, choice)) {
                return self.node(node, fit, path, key);
            }
            // Report against the alternative of the node's own kind, which
            // says more than "matches none".
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
            if !consts.is_empty() {
                self.problems.push(
                    Problem::new(format!("`{}` must be {}", dotted(path), or_list(&consts)))
                        .at(at)
                        .label(format!("found {}", node.kind()))
                        .help(choice_help(self, choices)),
                );
            } else {
                let kinds: Vec<&str> = choices
                    .iter()
                    .flat_map(|c| Self::types(self.resolve(c)))
                    .collect();
                self.problems.push(
                    Problem::new(format!("`{}` must be {}", dotted(path), wanted(&kinds)))
                        .at(at)
                        .label(format!("found {}", node.kind())),
                );
            }
            return None;
        }

        if let Some(values) = schema.get("enum").and_then(Json::as_array) {
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
            return None;
        }
        if let Some(constant) = schema.get("const").and_then(Json::as_str) {
            if matches!(&node.value, Value::Scalar { text, .. } if text == constant) {
                return Some(Json::String(constant.to_string()));
            }
            self.problems.push(
                Problem::new(format!("`{}` must be {constant}", dotted(path)))
                    .at(at)
                    .label(format!("found {}", node.kind())),
            );
            return None;
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
            Value::Seq(items) if types.contains(&"array") => {
                let items_schema = schema.get("items").unwrap_or(&Json::Bool(true));
                let mut out = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    path.push(format!("[{index}]"));
                    if let Some(value) = self.node(item, items_schema, path, Some(item.span)) {
                        out.push(value);
                    }
                    path.pop();
                }
                Some(Json::Array(out))
            }
            Value::Scalar { text, plain, tag } if !node.is_null() => {
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
                if types.contains(&"boolean") && *plain && matches!(text.as_str(), "true" | "false")
                {
                    return Some(Json::Bool(text == "true"));
                }
                self.mismatch(node, schema, &types, path, at);
                None
            }
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
        // `run: npm test` instead of `run: { local: npm test }`.
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
                let last = path.last().map(String::as_str).unwrap_or("run");
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
            match schema_for {
                Some(child) => {
                    // A key with nothing after it means "the default".
                    if value.is_null() && property.is_some() && !required(schema, &key.text) {
                        path.pop();
                        continue;
                    }
                    if let Some(json) = self.node(value, child, path, Some(key.span)) {
                        out.insert(key.text.clone(), json);
                    }
                }
                None => {
                    let known: Vec<&str> = properties
                        .map(|p| p.keys().map(String::as_str).collect())
                        .unwrap_or_default();
                    let parent = dotted(&path[..path.len() - 1]);
                    if MOVED.iter().any(|(p, k, _)| *p == parent && *k == key.text) {
                        self.old_format = true;
                    }
                    self.problems.push(unknown_key(path, &known, key.span));
                }
            }
            path.pop();
        }
        // A 0.2 file misses 0.3 keys because they moved; that is said already.
        if let Some(required) = schema
            .get("required")
            .and_then(Json::as_array)
            .filter(|_| !self.old_format)
        {
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

/// Plain YAML as JSON, for parts the schema leaves open.
fn plain_json(node: &Node) -> Json {
    match &node.value {
        _ if node.is_null() => Json::Null,
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

/// The steps shipdeck runs itself, with where their commands belong instead.
const SHIPDECK_STEPS: [(&str, &str); 4] = [
    (
        "upload",
        "commands before the upload go in `pipeline.build.run.local`",
    ),
    (
        "unpack",
        "commands for the unpacked release go in `pipeline.setup.run.server`",
    ),
    (
        "switch",
        "commands before the switch go in `pipeline.setup.run.server`; after it, in `pipeline.done.run`",
    ),
    (
        "restart",
        "checks after the restart go in `pipeline.health.check`",
    ),
];

/// Keys from shipdeck 0.2, with where they live now.
const MOVED: [(&str, &str, &str); 14] = [
    (
        "",
        "remote",
        "it is `service` now: name, dir, start, run_as, env, env_file",
    ),
    (
        "",
        "hooks",
        "each step has its own commands now: `pipeline.preflight.run.local`, `pipeline.setup.run.server`, `pipeline.done.run.local`",
    ),
    (
        "",
        "build",
        "it is `pipeline.build` now (run.local, version_from); binary and files moved to `pipeline.pack`",
    ),
    (
        "",
        "bundle",
        "it is `pipeline.pack.files.include.deploy` now",
    ),
    ("", "releases", "it is `service.keep` now"),
    ("", "health", "it is `pipeline.health` now"),
    ("service", "service", "the unit's name is `service.name`"),
    (
        "service",
        "args",
        "put the arguments at the end of `service.start`",
    ),
    (
        "service",
        "requires",
        "it is `pipeline.preflight.requires` now",
    ),
    (
        "pipeline.build",
        "command",
        "it is `pipeline.build.run.local` now",
    ),
    (
        "pipeline.build",
        "binary",
        "it is `pipeline.pack.binary` now",
    ),
    (
        "pipeline.build",
        "files",
        "it is `pipeline.pack.files.include.project` now",
    ),
    (
        "pipeline.build",
        "cwd",
        "it is `project` now, at the top of the file",
    ),
    (
        "pipeline",
        "rotate",
        "old releases are tidied in done; `service.keep` says how many stay",
    ),
];

fn unknown_key(path: &[String], known: &[&str], at: Span) -> Problem {
    let key = path.last().map(String::as_str).unwrap_or_default();
    let parent = dotted(&path[..path.len() - 1]);
    let full = dotted(path);

    if parent == "pipeline"
        && let Some((step, help)) = SHIPDECK_STEPS.iter().find(|(step, _)| *step == key)
    {
        return Problem::new(format!("{step} does not take commands"))
            .at(Some(at))
            .label(format!("{step} is done by shipdeck"))
            .help(*help);
    }
    if let Some((_, _, help)) = MOVED.iter().find(|(p, k, _)| *p == parent && *k == key) {
        return Problem::new(format!("`{full}` moved in shipdeck 0.3"))
            .at(Some(at))
            .label("a 0.2 key")
            .help(*help);
    }

    // `pipeline.build.run.server` and friends.
    let segments: Vec<&str> = path.iter().map(String::as_str).collect();
    if let [
        "pipeline",
        step,
        "run" | "check",
        side @ ("local" | "server"),
    ] = segments.as_slice()
    {
        let (where_, only) = if *side == "server" {
            ("on the server", "on your PC only")
        } else {
            ("on your PC", "on the server only")
        };
        let help = match (*step, *side) {
            ("build", _) => "the server sets up the release in `pipeline.setup.run.server`",
            ("setup", _) => {
                "commands on your PC go in `pipeline.preflight.run.local` or `pipeline.build.run.local`"
            }
            ("failed", _) => "the server may be out of reach when a step fails",
            _ => "",
        };
        let mut problem = Problem::new(format!("{step} cannot run commands {where_}"))
            .at(Some(at))
            .label(format!("{step} runs {only}"));
        if !help.is_empty() {
            problem = problem.help(help);
        }
        return problem;
    }
    if let ["pipeline", step, "run" | "check"] = segments.as_slice() {
        let help = match (*step, key) {
            ("health", "run") => {
                "health takes `check`: commands that must pass for the release to count as healthy"
                    .to_string()
            }
            (_, "check") if known.contains(&"run") => {
                format!("{step} takes `run`; a failing command already stops it")
            }
            _ => format!("{step} takes {}", known_list(known)),
        };
        return Problem::new(format!("{step} has no `{key}`"))
            .at(Some(at))
            .label(format!("not a field of {step}"))
            .help(help);
    }
    if let ["pipeline", "pack", "files", "include" | "exclude", _] = segments.as_slice() {
        return Problem::new(format!("unknown key `{full}`"))
            .at(Some(at))
            .label("not a source")
            .help("sources are project and deploy");
    }

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
    let named: Vec<&str> = known.to_vec();
    or_and(&named, "and")
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
        "name" => "add `name: My API (prod)` at the top".into(),
        "ssh" => "add `ssh: { host: my-server, user: deploy }`".into(),
        "ssh.host" => "add `host:` with the server's address or a ~/.ssh/config alias".into(),
        "service" => "add `service: { name: my-api, dir: /opt/my-api }`".into(),
        "service.name" => "add `name: my-api` (the unit becomes my-api.service)".into(),
        "service.dir" => "add `dir: /opt/my-api`".into(),
        other => format!("add `{}:`", other.rsplit('.').next().unwrap_or(other)),
    }
}
