//! Every value check a deploy file needs after its shape is known: required
//! and refused keys, one manager, reserved properties, names and paths.

use super::names::{is_plain_name, is_program, is_safe_dir};
use super::paths;
use super::placeholders::{self, Scope};
use super::problem::{Problem, Span};
use super::types::DeployFile;

mod app_rules;
mod file_rules;
use super::yaml::{Node, Value};

/// The systemd properties shipdeck writes itself.
const SYSTEMD_OWN: [(&str, &str); 5] = [
    ("ExecStart", "it comes from `process.start` or `binary`"),
    ("WorkingDirectory", "the process runs in the app folder"),
    ("User", "use `systemd.user`"),
    ("Environment", "use `process.env` or `process.env_file`"),
    ("EnvironmentFile", "use `process.env_file`"),
];

/// The PM2 properties shipdeck writes itself.
const PM2_OWN: [(&str, &str); 6] = [
    ("name", "use `process.name`"),
    ("cwd", "the process runs in the app folder"),
    ("script", "it comes from `process.start` or `binary`"),
    ("args", "they come from `process.start`"),
    ("interpreter", "it comes from `process.start`"),
    ("env", "use `process.env` or `process.env_file`"),
];

/// Checks a deploy file, with its lines when the YAML tree is given.
#[must_use]
pub fn check(deploy: &DeployFile, root: Option<&Node>) -> Vec<Problem> {
    let mut rules = Rules {
        deploy,
        root,
        problems: Vec::new(),
    };
    rules.top();
    rules.release();
    rules.apps();
    rules.placeholders();
    rules.problems
}

/// Whether a deploy file passes the value checks, without its lines.
#[must_use]
pub fn valid(deploy: &DeployFile) -> bool {
    check(deploy, None).is_empty()
}

struct Rules<'a> {
    deploy: &'a DeployFile,
    root: Option<&'a Node>,
    problems: Vec<Problem>,
}

impl Rules<'_> {
    /// Where `path` was written, or its nearest parent that was.
    fn span(&self, path: &[&str]) -> Option<Span> {
        let root = self.root?;
        (1..=path.len())
            .rev()
            .find_map(|len| root.span_of(&path[..len]))
    }

    fn fail(&mut self, path: &[&str], message: impl Into<String>, label: &str, help: &str) {
        let mut problem = Problem::new(message).at(self.span(path)).label(label);
        if !help.is_empty() {
            problem = problem.help(help);
        }
        self.problems.push(problem);
    }

    fn top(&mut self) {
        let name = &self.deploy.name;
        if name.trim().is_empty() {
            self.fail(&["name"], "`name` is empty", "empty", "such as `shop`");
        } else if !name.contains("{{") && (!is_plain_name(name) || name.starts_with('.')) {
            self.fail(
                &["name"],
                "`name` is not a plain name",
                "not allowed",
                "letters, digits, `.`, `_` and `-`, not starting with `.`",
            );
        }
        if !is_safe_dir(&self.deploy.server.dir) {
            self.fail(
                &["server", "dir"],
                "`server.dir` must be an absolute, safe folder",
                "not allowed",
                "such as /opt/shop, with no `..`",
            );
        }
        if self.deploy.server.ssh.host.trim().is_empty() {
            self.fail(&["server", "ssh", "host"], "`host` is empty", "empty", "");
        }
        // A `${NAME}` is checked once it is replaced.
        if let Some(user) = &self.deploy.server.ssh.user
            && !user.contains("${")
            && !is_plain_name(user)
        {
            self.fail(
                &["server", "ssh", "user"],
                "`server.ssh.user` is not a user name",
                "not allowed",
                "letters, digits, `.`, `_` and `-`",
            );
        }
        if self.deploy.apps.is_empty() {
            self.fail(
                &["apps"],
                "a deploy needs at least one app",
                "no apps",
                "add `apps: { web: ... }`",
            );
        }
        let mut processes: Vec<String> = Vec::new();
        for (name, app) in &self.deploy.apps {
            if !is_plain_name(name) || name.starts_with('.') {
                self.fail(
                    &["apps", name],
                    format!("app name {name:?} is not a plain name"),
                    "not allowed",
                    "letters, digits, `.`, `_` and `-`, not starting with `.`",
                );
            }
            if app.process.is_some() {
                let process = self.process_name(name);
                if process.contains('@') {
                    self.fail(
                        &["apps", name, "process", "name"],
                        format!("process name {process:?} holds `@`, which systemd reserves"),
                        "reserved",
                        "systemd instances are planned, not supported yet",
                    );
                }
                if processes.contains(&process) {
                    self.fail(
                        &["apps", name, "process", "name"],
                        format!("a process named {process:?} already exists"),
                        "twice",
                        "give one app's process another name",
                    );
                } else {
                    processes.push(process);
                }
            }
        }
    }

    fn release(&mut self) {
        let release = &self.deploy.release;
        self.files(&[], &release.files, &release.exclude, &release.shared, None);
        self.shared(&["release"], &release.shared);
        let steps = &release.steps;
        for tool in &steps.preflight.requires {
            if !is_program(tool) {
                self.fail(
                    &["release", "steps", "preflight", "requires"],
                    format!("{tool:?} is not a program name"),
                    "not a program",
                    "a name such as `node`, or an absolute path",
                );
            }
        }
        if let Some(file) = &steps.build.version_from
            && file.trim().is_empty()
        {
            self.fail(
                &["release", "steps", "build", "version_from"],
                "`version_from` is empty",
                "empty",
                "a file relative to the project folder",
            );
        }
        self.health(
            &["release", "steps", "health"],
            Some(&steps.health),
            self.deploy.release.steps.health.url.is_some(),
        );
        let keep = steps.backup.keep.unwrap_or_default();
        if keep.open < 1 {
            self.fail(
                &["release", "steps", "backup", "keep", "open"],
                "`keep.open` must be at least 1; the live release counts",
                "too small",
                "",
            );
        }
        self.patterns(
            &["release", "steps", "backup", "include"],
            &steps.backup.include,
        );
        self.patterns(
            &["release", "steps", "backup", "exclude"],
            &steps.backup.exclude,
        );
        self.commands(
            &["release", "steps"],
            [
                ("preflight", &steps.preflight.local, &steps.preflight.server),
                ("build", &steps.build.local, &steps.build.server),
                ("upload", &steps.upload.local, &steps.upload.server),
                ("setup", &steps.setup.local, &steps.setup.server),
                ("backup", &steps.backup.local, &steps.backup.server),
                ("switch", &steps.switch.local, &steps.switch.server),
                ("restart", &steps.restart.local, &steps.restart.server),
                ("nginx", &steps.nginx.local, &steps.nginx.server),
                ("health", &steps.health.local, &steps.health.server),
                ("done", &steps.done.local, &steps.done.server),
                ("rollback", &steps.rollback.local, &steps.rollback.server),
                ("failed", &steps.failed.local, &steps.failed.server),
            ],
        );
    }

    fn apps(&mut self) {
        for (name, app) in &self.deploy.apps {
            self.app(name, app);
        }
    }
}

impl Rules<'_> {
    fn process_name(&self, app: &str) -> String {
        self.deploy
            .apps
            .get(app)
            .and_then(|app| app.process.as_ref())
            .and_then(|process| process.name.clone())
            .unwrap_or_else(|| format!("{}-{app}", self.deploy.name))
    }

    /// Every placeholder in every string, with the scope of its place.
    fn placeholders(&mut self) {
        let Some(root) = self.root else {
            return;
        };
        let mut path = Vec::new();
        self.walk(root, &mut path);
    }

    fn walk(&mut self, node: &Node, path: &mut Vec<String>) {
        match &node.value {
            Value::Scalar { text, .. } => {
                let scope = self.scope(path);
                if let Err(bad) = placeholders::check(text, scope) {
                    let mut problem = Problem::new(bad.message).at(Some(node.span)).label("here");
                    if let Some(help) = bad.help {
                        problem = problem.help(help);
                    }
                    self.problems.push(problem);
                }
            }
            Value::Seq(items) => {
                for item in items {
                    self.walk(item, path);
                }
            }
            Value::Map(entries) => {
                for (key, child) in entries {
                    path.push(key.text.clone());
                    self.walk(child, path);
                    path.pop();
                }
            }
        }
    }

    /// Where a string at `path` is, and which names have a value there.
    fn scope(&self, path: &[String]) -> Scope {
        let in_app = path.first().is_some_and(|part| part == "apps") && path.len() > 1;
        let app_name = in_app.then(|| path[1].as_str());
        let app_has_process = app_name
            .and_then(|name| self.deploy.apps.get(name))
            .is_some_and(|app| app.process.is_some());
        let preflight = path
            .windows(2)
            .any(|pair| pair[0] == "steps" && pair[1] == "preflight");
        let early = preflight
            || path.first().is_some_and(|part| part == "server")
            || path.first().is_some_and(|part| part == "name");
        let release = !early;
        if in_app {
            Scope::app(app_has_process, release)
        } else if release {
            Scope::RELEASE
        } else {
            Scope::EARLY
        }
    }
}

/// A YAML option value a unit line can carry: text, a number or a list.
pub(super) fn is_option_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null
        | serde_json::Value::String(_)
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_) => true,
        serde_json::Value::Array(items) => items.iter().all(|item| {
            matches!(
                item,
                serde_json::Value::String(_)
                    | serde_json::Value::Number(_)
                    | serde_json::Value::Bool(_)
            )
        }),
        serde_json::Value::Object(_) => false,
    }
}

/// `a` when `path` is `a` or inside it; `path` itself when `a` is empty.
pub(super) fn inside(base: &str, path: &str) -> bool {
    if base.is_empty() || base == "." {
        return true;
    }
    path == base || path.starts_with(&format!("{base}/"))
}

/// The first part of a pattern that has no wildcard.
pub(super) fn wildcard_base(text: &str) -> String {
    let mut parts = Vec::new();
    for part in text.split('/') {
        if paths::has_wildcard(part) {
            break;
        }
        parts.push(part);
    }
    parts.join("/")
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
