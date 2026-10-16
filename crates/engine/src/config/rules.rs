//! The second look at a deploy.yaml: values that have the right shape but
//! would be unsafe or ambiguous once they reach a remote shell, a systemd
//! unit or the release folder.

use std::path::Path;

use super::glob::Pattern;
use super::problem::{Problem, Span};
use super::yaml::Node;
use super::{
    Commands, DeployConfig, Manager, Program, file_name, is_ecosystem_file, is_node, is_plain_name,
    is_program, is_unit_name, release_path, split_args,
};
use crate::release::MANIFEST_FILE;

pub fn check(config: &DeployConfig, root: &Node) -> Vec<Problem> {
    let mut rules = Rules {
        root,
        problems: Vec::new(),
    };
    rules.top(config);
    rules.service(config);
    rules.pm2(config);
    rules.pipeline(config);
    rules.backup(config);
    rules.files(config);
    rules.problems
}

struct Rules<'a> {
    root: &'a Node,
    problems: Vec<Problem>,
}

impl Rules<'_> {
    /// Where `path` was written, or its nearest parent that was.
    fn span(&self, path: &[&str]) -> Option<Span> {
        (1..=path.len())
            .rev()
            .find_map(|len| self.root.span_of(&path[..len]))
    }

    fn fail(&mut self, path: &[&str], message: impl Into<String>, label: &str, help: &str) {
        let mut problem = Problem::new(message).at(self.span(path)).label(label);
        if !help.is_empty() {
            problem = problem.help(help);
        }
        self.problems.push(problem);
    }

    fn top(&mut self, config: &DeployConfig) {
        if config.name.trim().is_empty() {
            self.fail(
                &["name"],
                "`name` is empty",
                "empty",
                "such as `My API (prod)`",
            );
        } else if config.name.contains(['\n', '\r']) {
            self.fail(&["name"], "`name` must be one line", "several lines", "");
        }
        if config.project.trim().is_empty() {
            self.fail(
                &["project"],
                "`project` is empty",
                "empty",
                "a folder relative to this deploy.yaml, such as `../../../web`; leave it out for this folder",
            );
        }
        if config.ssh.host.trim().is_empty() {
            self.fail(&["ssh", "host"], "`ssh.host` is empty", "empty", "");
        }
        if let Some(user) = &config.ssh.user
            && !is_plain_name(user)
        {
            self.fail(
                &["ssh", "user"],
                "`ssh.user` is not a user name",
                "not allowed",
                "letters, digits, `.`, `_` and `-` only",
            );
        }
    }

    fn service(&mut self, config: &DeployConfig) {
        let service = &config.service;
        let name = &service.name;
        if name.ends_with(".service") {
            self.fail(
                &["service", "name"],
                "`service.name` is the name without .service",
                "drop .service",
                "",
            );
        } else if !is_unit_name(name) {
            self.fail(
                &["service", "name"],
                "`service.name` is not a unit name",
                "not allowed",
                "letters, digits, `.`, `_`, `@` and `-` only",
            );
        }

        let dir = &service.dir;
        if !(dir.starts_with('/') && dir.len() > 1) {
            self.fail(
                &["service", "dir"],
                "`service.dir` must be an absolute path",
                "not absolute",
                "such as /opt/my-api",
            );
        } else if !dir
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
            || dir.split('/').any(|part| part == "..")
        {
            self.fail(
                &["service", "dir"],
                "`service.dir` may only use letters, digits, `.`, `_`, `-` and `/`",
                "not allowed",
                "and no `..`",
            );
        }

        let ecosystem = service.manager == Manager::Pm2 && service.pm2.ecosystem.is_some();
        match &service.start {
            None if config.pipeline.pack.binary.is_none() && !ecosystem => self.fail(
                &["service"],
                "`service.start` is needed when there is no `pipeline.pack.binary`",
                "what does the service run?",
                "such as `start: node dist/main.js`, or `pm2.ecosystem` with `manager: pm2`",
            ),
            None => {}
            Some(_) if ecosystem => self.fail(
                &["service", "start"],
                "`service.start` and `pm2.ecosystem` both say what runs",
                "pick one",
                "keep `pm2.ecosystem` (your file runs the app), or `service.start` (shipdeck writes the file)",
            ),
            Some(start) => {
                let program = start.split_whitespace().next().unwrap_or_default();
                if start.trim().is_empty() {
                    self.fail(
                        &["service", "start"],
                        "`service.start` is empty",
                        "empty",
                        "",
                    );
                } else if start.contains(['\n', '\r', '\\']) {
                    self.fail(
                        &["service", "start"],
                        "`service.start` must be one line without backslashes",
                        "not allowed",
                        "",
                    );
                } else if split_args(start).is_none() {
                    self.fail(
                        &["service", "start"],
                        "`service.start` has a quote that is never closed",
                        "unclosed quote",
                        "quote an argument with spaces like `--title \"my api\"`",
                    );
                } else if !is_program(program) {
                    self.fail(
                        &["service", "start"],
                        format!("`service.start` must begin with a program, not {program:?}"),
                        "not a program",
                        "a name such as `node`, or a path such as `./my-api` or `/usr/bin/node`",
                    );
                }
            }
        }
        if let Some(user) = &service.run_as
            && !is_plain_name(user)
        {
            self.fail(
                &["service", "run_as"],
                "`service.run_as` is not a user name",
                "not allowed",
                "letters, digits, `.`, `_` and `-` only",
            );
        }
        if let Some(file) = &service.env_file
            && !(is_program(file) && !file.starts_with('/'))
        {
            self.fail(
                &["service", "env_file"],
                "`service.env_file` must be a path inside the release",
                "not inside the release",
                "such as `.env`, shipped with `pipeline.pack.files.include.deploy: [.env]`",
            );
        }
        for (key, value) in &service.env {
            let valid = !key.is_empty()
                && !key.starts_with(|c: char| c.is_ascii_digit())
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid {
                self.fail(
                    &["service", "env", key],
                    format!("`service.env` key {key:?} is not a variable name"),
                    "not a variable name",
                    "letters, digits and `_`, not starting with a digit",
                );
            } else if value.contains(['"', '\\', '\n', '\r']) {
                self.fail(
                    &["service", "env", key],
                    format!("`service.env.{key}` must not contain quotes, backslashes or newlines"),
                    "not allowed",
                    "put values like this in `service.env_file`",
                );
            }
        }
        let mut seen: Vec<String> = Vec::new();
        for (index, entry) in service.shared.iter().enumerate() {
            let at = ["service", "shared", &index.to_string()];
            let stored = release_path(entry);
            if entry.starts_with('/') || !is_program(entry) || stored.is_empty() {
                self.fail(
                    &at,
                    format!("`service.shared` entry {entry:?} must be a folder inside the release"),
                    "not allowed",
                    "such as `uploads` or `data/cache`",
                );
            } else if reserved(config).contains(&stored) {
                self.fail(&at, format!("{stored} belongs to shipdeck"), "reserved", "");
            } else if let Some(other) = seen.iter().find(|other| overlaps(other, &stored)).cloned()
            {
                self.fail(
                    &at,
                    format!("`service.shared` lists {stored:?} and {other:?}, which overlap"),
                    "overlaps",
                    "keep the outer folder",
                );
            } else {
                seen.push(stored);
            }
        }
    }

    fn pm2(&mut self, config: &DeployConfig) {
        let service = &config.service;
        let pm2 = &service.pm2;
        if service.manager != Manager::Pm2 {
            if self.root.span_of(&["service", "pm2"]).is_some() {
                self.fail(
                    &["service", "pm2"],
                    "`service.pm2` is only read with `manager: pm2`",
                    "not used",
                    "add `manager: pm2` to `service`, or remove this",
                );
            }
            return;
        }
        if let Some(user) = &service.run_as
            && *user != config.ssh.user.clone().unwrap_or_default()
        {
            self.fail(
                &["service", "run_as"],
                "PM2 runs the app as the SSH user, so `service.run_as` cannot apply",
                "not with PM2",
                "remove it, or log in as that user with `ssh.user`",
            );
        }
        let app = service.pm2_app();
        let at: &[&str] = if pm2.app.is_some() {
            &["service", "pm2", "app"]
        } else {
            &["service", "name"]
        };
        if !is_plain_name(app) {
            self.fail(
                at,
                format!("{app:?} is not a PM2 app name"),
                "not allowed",
                "letters, digits, `.`, `_` and `-` only",
            );
        } else if app.chars().all(|c| c.is_ascii_digit()) {
            self.fail(
                at,
                format!("{app:?} is not a PM2 app name"),
                "PM2 reads a number as a process id",
                "use a name with a letter in it",
            );
        }
        if let Some(file) = &pm2.ecosystem {
            let at = ["service", "pm2", "ecosystem"];
            if file.starts_with('/') || !is_program(file) || release_path(file).is_empty() {
                self.fail(
                    &at,
                    "`pm2.ecosystem` must be a file inside the release",
                    "not inside the release",
                    "such as `ecosystem.config.js`, shipped with `pipeline.pack.files`",
                );
            } else if !is_ecosystem_file(file) {
                self.fail(
                    &at,
                    format!("PM2 does not read {file:?} as an ecosystem file"),
                    "not an ecosystem file",
                    "its name must end in .config.js, .config.cjs, .json, .yml or .yaml",
                );
            }
            if !pm2.options.is_empty() {
                self.fail(
                    &["service", "pm2", "options"],
                    "`pm2.options` and `pm2.ecosystem` are both set",
                    "pick one",
                    "keep `pm2.ecosystem` (your file, deploy.yaml settings ignored), or `pm2.options` (shipdeck writes the file)",
                );
            }
        }
        if pm2.ecosystem.is_none()
            && let Some(start) = &service.start
            && let Some(words) = split_args(start)
            && let [program, first, ..] = words.as_slice()
            && is_node(&Program::Tool(program.clone()))
            && first.starts_with('-')
        {
            self.fail(
                &["service", "start"],
                format!("with PM2, node's first argument is the script, not {first:?}"),
                "a node flag",
                "move node's flags to `pm2.options.node_args`, such as `node_args: [--enable-source-maps]`",
            );
        }
        for key in pm2.options.keys() {
            if let Some((_, help)) = PM2_OWN_OPTIONS.iter().find(|(own, _)| own == key) {
                self.fail(
                    &["service", "pm2", "options", key],
                    format!("shipdeck sets the PM2 `{key}` itself"),
                    "set by shipdeck",
                    help,
                );
            }
        }
    }

    fn pipeline(&mut self, config: &DeployConfig) {
        let pipeline = &config.pipeline;
        for (index, tool) in pipeline.preflight.requires.iter().enumerate() {
            if !is_program(tool) {
                self.fail(
                    &["pipeline", "preflight", "requires", &index.to_string()],
                    format!("{tool:?} is not a program name"),
                    "not a program",
                    "a name such as `node`, or an absolute path",
                );
            }
        }
        let commands: [(&[&str], &Commands); 10] = [
            (
                &["pipeline", "preflight", "run", "local"],
                &pipeline.preflight.run.local,
            ),
            (
                &["pipeline", "preflight", "run", "server"],
                &pipeline.preflight.run.server,
            ),
            (
                &["pipeline", "build", "run", "local"],
                &pipeline.build.run.local,
            ),
            (
                &["pipeline", "setup", "run", "server"],
                &pipeline.setup.run.server,
            ),
            (
                &["pipeline", "setup", "check", "server"],
                &pipeline.setup.check.server,
            ),
            (
                &["pipeline", "health", "check", "local"],
                &pipeline.health.check.local,
            ),
            (
                &["pipeline", "health", "check", "server"],
                &pipeline.health.check.server,
            ),
            (
                &["pipeline", "done", "run", "local"],
                &pipeline.done.run.local,
            ),
            (
                &["pipeline", "done", "run", "server"],
                &pipeline.done.run.server,
            ),
            (
                &["pipeline", "failed", "run", "local"],
                &pipeline.failed.run.local,
            ),
        ];
        for (path, list) in commands {
            for (index, command) in list.iter().enumerate() {
                if command.trim().is_empty() {
                    let mut at = path.to_vec();
                    let index = index.to_string();
                    at.push(&index);
                    self.fail(&at, "this command is empty", "empty", "remove it");
                }
            }
        }
        if let Some(file) = &pipeline.build.version_from
            && file.trim().is_empty()
        {
            self.fail(
                &["pipeline", "build", "version_from"],
                "`pipeline.build.version_from` is empty",
                "empty",
                "a file relative to `project`, such as `package.json`",
            );
        }

        let health = &pipeline.health;
        if let Some(url) = &health.url {
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                self.fail(
                    &["pipeline", "health", "url"],
                    "`pipeline.health.url` must start with http:// or https://",
                    "not a URL",
                    "such as http://127.0.0.1:3000/health",
                );
            } else if url.contains(['\'', '"', '\n', '\r', ' ']) {
                self.fail(
                    &["pipeline", "health", "url"],
                    "`pipeline.health.url` must not contain quotes, spaces or newlines",
                    "not allowed",
                    "",
                );
            }
        }
        if health.timeout_secs < 3 {
            self.fail(
                &["pipeline", "health", "timeout_secs"],
                "`pipeline.health.timeout_secs` must be at least 3",
                "too short",
                "a release counts as up after 3 healthy seconds",
            );
        }
    }

    fn backup(&mut self, config: &DeployConfig) {
        let backup = &config.pipeline.backup;
        if backup.keep.open < 1 {
            self.fail(
                &["pipeline", "backup", "keep", "open"],
                "`pipeline.backup.keep.open` must be at least 1",
                "the live release counts",
                "",
            );
        }
        let files = &backup.files;
        let shared: Vec<String> = config
            .service
            .shared
            .iter()
            .map(|s| release_path(s))
            .collect();
        for (list, entries) in [("include", &files.include), ("exclude", &files.exclude)] {
            let mut literals: Vec<String> = Vec::new();
            for (index, text) in entries.iter().enumerate() {
                let index = index.to_string();
                let at = ["pipeline", "backup", "files", list, &index];
                let pattern = match Pattern::parse(text) {
                    Ok(pattern) => pattern,
                    Err(why) => {
                        let help = if why.contains("..") {
                            "a pattern stays inside the release"
                        } else {
                            ""
                        };
                        self.fail(&at, format!("{text:?} {why}"), "not a usable pattern", help);
                        continue;
                    }
                };
                let Some(literal) = pattern.literal() else {
                    continue;
                };
                if literals.contains(&literal) {
                    self.fail(
                        &at,
                        format!("{text:?} is listed twice"),
                        "again",
                        "remove one",
                    );
                } else if list == "include"
                    && let Some(shared) = shared
                        .iter()
                        .find(|shared| !literal.is_empty() && overlaps(shared, &literal))
                {
                    self.fail(
                        &at,
                        format!("{literal:?} is in `service.shared`, so no backup holds it"),
                        "shared across releases",
                        &format!("{shared} lives in shared/{shared}, outside every release"),
                    );
                } else {
                    literals.push(literal);
                }
            }
        }
        let brings_something = files.deployed || files.include.iter().any(|p| !p.starts_with('!'));
        if !brings_something && backup.keep.compressed > 0 {
            self.fail(
                &["pipeline", "backup", "files"],
                "the backup would hold nothing but release.json and the service file",
                "nothing to back up",
                "set `deployed: true`, list paths in `include`, or keep no backups with `keep.compressed: 0`",
            );
        }
    }

    fn files(&mut self, config: &DeployConfig) {
        let pack = &config.pipeline.pack;
        let files = &pack.files;
        let reserved = reserved(config);
        if let Some(binary) = &pack.binary {
            if binary.trim().is_empty() {
                self.fail(
                    &["pipeline", "pack", "binary"],
                    "`pipeline.pack.binary` is empty",
                    "empty",
                    "",
                );
            } else if reserved.contains(&file_name(binary)) {
                self.fail(
                    &["pipeline", "pack", "binary"],
                    format!("{} belongs to shipdeck", file_name(binary)),
                    "reserved",
                    "rename the binary",
                );
            }
        }
        let ships_something = pack.binary.is_some()
            || [&files.include.project, &files.include.deploy]
                .iter()
                .any(|list| list.iter().any(|p| !p.starts_with('!')));
        if !ships_something {
            self.fail(
                &["pipeline", "pack"],
                "nothing to ship",
                "empty",
                "set `pipeline.pack.binary` (a compiled program), `pipeline.pack.files.include` (build output such as dist), or both",
            );
        }

        for (list, source, entries) in [
            ("include", "project", &files.include.project),
            ("include", "deploy", &files.include.deploy),
            ("exclude", "project", &files.exclude.project),
            ("exclude", "deploy", &files.exclude.deploy),
        ] {
            let mut literals: Vec<String> = Vec::new();
            for (index, text) in entries.iter().enumerate() {
                let index = index.to_string();
                let at = ["pipeline", "pack", "files", list, source, &index];
                let pattern = match Pattern::parse(text) {
                    Ok(pattern) => pattern,
                    Err(why) => {
                        let help = if why.contains("..") {
                            "a pattern stays inside its source; move the file, or point `project` higher"
                        } else {
                            ""
                        };
                        self.fail(&at, format!("{text:?} {why}"), "not a usable pattern", help);
                        continue;
                    }
                };
                let Some(literal) = pattern.literal() else {
                    continue;
                };
                if list == "include" && reserved.contains(&literal) {
                    self.fail(
                        &at,
                        format!("{literal} belongs to shipdeck"),
                        "reserved",
                        "",
                    );
                } else if list == "include" && literal.is_empty() && source == "deploy" {
                    // `.` is fine: deploy.yaml itself is always left out.
                } else if literals.contains(&literal) {
                    self.fail(
                        &at,
                        format!("{text:?} is listed twice"),
                        "again",
                        "remove one",
                    );
                } else if list == "include"
                    && let Some(shared) = config
                        .service
                        .shared
                        .iter()
                        .map(|s| release_path(s))
                        .find(|shared| !literal.is_empty() && overlaps(shared, &literal))
                {
                    self.fail(
                        &at,
                        format!("{literal:?} is in `service.shared`, so it cannot be shipped too"),
                        "shared across releases",
                        &format!("every release links {shared} to shared/{shared}; leave it out of the pack"),
                    );
                } else {
                    literals.push(literal);
                }
            }
        }
    }
}

/// The files shipdeck writes into every release.
fn reserved(config: &DeployConfig) -> Vec<String> {
    let mut files = vec![MANIFEST_FILE.to_string()];
    files.extend(config.service.own_file());
    files
}

/// The settings shipdeck writes into the PM2 app itself.
const PM2_OWN_OPTIONS: [(&str, &str); 3] = [
    ("name", "set `service.pm2.app`"),
    ("cwd", "the app runs in `current`"),
    ("env", "set `service.env` or `service.env_file`"),
];

/// `a` and `a/b` overlap; `a` and `ab` do not.
fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (Path::new(a), Path::new(b));
    a.starts_with(b) || b.starts_with(a)
}

#[cfg(test)]
#[path = "rules_test.rs"]
mod tests;
