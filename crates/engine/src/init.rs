//! `shipdeck init`: a new deployable folder under the deploy root, with a
//! `deploy.yaml` written from a preset. What it writes goes through the same
//! checks as a launch before anything touches the disk.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{Deployable, FILE_NAMES, FileProblems, Manager, config_file, schema};

/// The `.env` every preset ships; it starts empty and stays out of git.
const ENV_FILE: &str = ".env";

/// What the app is written in, and so how it is built and run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Node,
    Go,
    Java,
    Kotlin,
    Rust,
    Dart,
    Python,
    Bun,
}

impl Preset {
    pub const ALL: [Preset; 8] = [
        Preset::Node,
        Preset::Go,
        Preset::Java,
        Preset::Kotlin,
        Preset::Rust,
        Preset::Dart,
        Preset::Python,
        Preset::Bun,
    ];

    /// As `--preset` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Node => "nodejs",
            Preset::Go => "golang",
            Preset::Java => "java",
            Preset::Kotlin => "kotlin",
            Preset::Rust => "rust",
            Preset::Dart => "dart",
            Preset::Python => "python",
            Preset::Bun => "bun",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }

    /// How the app is built and what reaches the server, in a few words.
    pub fn summary(self) -> &'static str {
        match self {
            Preset::Node => "built here; the server installs production dependencies",
            Preset::Go => "one static binary, nothing to install on the server",
            Preset::Java | Preset::Kotlin => "built with Gradle; the server needs Java",
            Preset::Rust => "one binary, nothing to install on the server",
            Preset::Dart => "compiled to one executable, nothing to install on the server",
            Preset::Python => "the source ships; the server builds its virtualenv",
            Preset::Bun => "compiled to one executable, nothing to install on the server",
        }
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Everything `init` needs to write a deployable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDeployable {
    /// The folder's name, also `name` and `service.name`.
    pub name: String,
    pub preset: Preset,
    pub manager: Manager,
    /// An address or an alias from `~/.ssh/config`.
    pub host: String,
    /// Left out to use the alias's `User`.
    pub user: Option<String>,
    /// The source folder, relative to the new folder.
    pub project: String,
}

impl NewDeployable {
    /// The `deploy.yaml` text.
    pub fn yaml(&self) -> String {
        let name = &self.name;
        let mut out = format!(
            "{modeline}\n# {comment}\nname: {q_name}\nproject: {project}\n\nssh:\n  host: {host}\n",
            modeline = schema::modeline(),
            comment = self.comment(),
            q_name = quote(name),
            project = quote(&self.project),
            host = quote(&self.host),
        );
        if let Some(user) = &self.user {
            out.push_str(&format!("  user: {}\n", quote(user)));
        }
        out.push_str(&format!(
            "\nservice:\n  name: {}\n  dir: {}\n",
            quote(name),
            quote(&format!("/opt/{name}"))
        ));
        if self.manager == Manager::Pm2 {
            out.push_str("  manager: pm2\n");
        }
        out.push_str(&self.service_lines());
        out.push_str(&format!("  env_file: {ENV_FILE}\n\npipeline:\n"));
        out.push_str(&self.pipeline());
        out
    }

    fn comment(&self) -> String {
        let what = match self.preset {
            Preset::Node => "A Node.js app",
            Preset::Go => "A Go service",
            Preset::Java => "A Java app",
            Preset::Kotlin => "A Kotlin app",
            Preset::Rust => "A Rust service",
            Preset::Dart => "A Dart server",
            Preset::Python => "A Python app",
            Preset::Bun => "A Bun app",
        };
        format!("{what}: {}.\n# Every field: {DOCS}", self.preset.summary())
    }

    /// `start` and `env` under `service`, when the preset needs them.
    fn service_lines(&self) -> String {
        let name = &self.name;
        match self.preset {
            Preset::Node => {
                "  start: node dist/main.js\n  env: { NODE_ENV: production }\n".to_string()
            }
            Preset::Java | Preset::Kotlin => {
                format!("  start: build/install/{name}/bin/{name}\n")
            }
            Preset::Python => "  start: .venv/bin/python main.py\n".to_string(),
            // `./<binary>` is the default.
            Preset::Go | Preset::Rust | Preset::Dart | Preset::Bun => String::new(),
        }
    }

    fn pipeline(&self) -> String {
        let name = &self.name;
        let binary = |build: &str, path: &str| {
            format!(
                "  build:\n    run:\n      local: {build}\n  pack:\n    binary: {path}\n    files:\n      include:\n        deploy: [{ENV_FILE}]\n"
            )
        };
        match self.preset {
            Preset::Node => format!(
                "  preflight:\n    requires: [npm]\n  build:\n    run:\n      local: [npm ci, npm run build]\n  pack:\n    files:\n      include:\n        project: [dist, package.json, package-lock.json]\n        deploy: [{ENV_FILE}]\n  setup:\n    run:\n      server: npm ci --omit=dev\n"
            ),
            Preset::Go => binary(
                &format!("CGO_ENABLED=0 GOOS=linux go build -o dist/{name} ."),
                &format!("dist/{name}"),
            ),
            Preset::Rust => binary("cargo build --release", &format!("target/release/{name}")),
            Preset::Dart => binary(
                &format!("[dart pub get, dart compile exe bin/server.dart -o dist/{name}]"),
                &format!("dist/{name}"),
            ),
            Preset::Bun => binary(
                &format!(
                    "[bun install, bun build ./src/index.ts --compile --target=bun-linux-x64 --outfile dist/{name}]"
                ),
                &format!("dist/{name}"),
            ),
            Preset::Java | Preset::Kotlin => format!(
                "  preflight:\n    requires: [java]\n  build:\n    run:\n      local: ./gradlew installDist\n  pack:\n    files:\n      include:\n        project: [build/install/{name}]\n        deploy: [{ENV_FILE}]\n"
            ),
            Preset::Python => format!(
                "  preflight:\n    requires: [python3]\n  pack:\n    files:\n      include:\n        project: [.]\n        deploy: [{ENV_FILE}]\n      exclude:\n        project: [.venv, \"**/__pycache__\", \"**/*.pyc\"]\n  setup:\n    run:\n      server:\n        - python3 -m venv .venv\n        - .venv/bin/pip install -q -r requirements.txt\n"
            ),
        }
    }
}

/// Where the README lists every field.
const DOCS: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "#deployyaml");

/// A YAML double-quoted scalar. JSON's string syntax is a subset of it.
fn quote(text: &str) -> String {
    serde_json::Value::String(text.to_string()).to_string()
}

/// Why `name` cannot be a new deployable's folder, if it cannot.
pub fn name_problem(root: &Path, name: &str) -> Option<String> {
    if !crate::config::is_unit_name(name) || name.starts_with('.') {
        return Some(
            "use letters, digits, `.`, `_`, `@` and `-`, not starting with `.`".to_string(),
        );
    }
    let folder = root.join(name);
    config_file(&folder).map(|file| format!("{} already exists", file.display()))
}

/// `project` as the new folder `root/name` sees it. Both paths must exist
/// except the new folder itself.
pub fn project_from(root: &Path, name: &str, project: &Path) -> Result<String> {
    let root = root
        .canonicalize()
        .with_context(|| format!("{} is not a folder", root.display()))?;
    let project = project
        .canonicalize()
        .with_context(|| format!("the project folder {} does not exist", project.display()))?;
    if !project.is_dir() {
        bail!("{} is not a folder", project.display());
    }
    Ok(relative(&root.join(name), &project)
        .to_string_lossy()
        .into_owned())
}

/// The path from the folder `from` to `to`; both absolute and without `..`.
pub fn relative(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut path = PathBuf::new();
    for _ in common..from.len() {
        path.push("..");
    }
    for part in &to[common..] {
        path.push(part);
    }
    if path.as_os_str().is_empty() {
        path.push(".");
    }
    path
}

/// The new deployable's `deploy.yaml`, checked as a launch would check it.
pub fn check(root: &Path, new: &NewDeployable) -> Result<String, FileProblems> {
    let folder = root.join(&new.name);
    let yaml = new.yaml();
    Deployable::from_text(&folder, folder.join(FILE_NAMES[0]), yaml.clone())?;
    Ok(yaml)
}

/// Writes the folder: `deploy.yaml`, an empty `.env` and a `.gitignore`
/// that keeps it out of git. Files already there are left alone, and an
/// existing `deploy.yaml` is never replaced. Returns the files it wrote.
pub fn write(root: &Path, name: &str, yaml: &str) -> Result<Vec<PathBuf>> {
    let folder = root.join(name);
    if let Some(file) = config_file(&folder) {
        bail!("{} already exists", file.display());
    }
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    let mut written = Vec::new();
    for (file, text) in [
        (FILE_NAMES[0], yaml),
        (ENV_FILE, ""),
        (".gitignore", ".env\n"),
    ] {
        let path = folder.join(file);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
#[path = "init_test.rs"]
mod tests;
