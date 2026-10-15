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
    /// The `deploy.yaml` text, with a short comment on each field.
    pub fn yaml(&self) -> String {
        let name = &self.name;
        let pm2 = self.manager == Manager::Pm2;
        let mut y = Yaml::default();
        y.raw(&schema::modeline());
        y.raw(&format!("# {}", self.comment()));
        y.raw(&format!("# Every field: {DOCS}"));
        y.raw("");
        y.line(
            0,
            &format!("name: {}", quote(name)),
            "shown in shipdeck and as the service's description",
        );
        y.line(
            0,
            &format!("project: {}", quote(&self.project)),
            "your source, relative to this file; local commands run there",
        );
        y.raw("");
        y.raw("ssh:");
        y.line(
            2,
            &format!("host: {}", quote(&self.host)),
            "an address, or an alias from ~/.ssh/config",
        );
        match &self.user {
            Some(user) => y.line(
                2,
                &format!("user: {}", quote(user)),
                "logs in and deploys as this user",
            ),
            None => y.line(2, "# user: deploy", "default: the alias's User"),
        }
        y.line(2, "# port: 22", "default: the alias's Port, else 22");
        y.raw("");
        y.raw("service:");
        let unit = if pm2 {
            "the PM2 app's name".to_string()
        } else {
            format!("the unit: {name}.service")
        };
        y.line(2, &format!("name: {}", quote(name)), &unit);
        y.line(
            2,
            &format!("dir: {}", quote(&format!("/opt/{name}"))),
            "on the server: current/, releases/, backups/, shared/",
        );
        if pm2 {
            y.line(
                2,
                "manager: pm2",
                "runs as the SSH user, no sudo; the server needs pm2",
            );
        } else {
            y.line(
                2,
                "manager: systemd",
                "or pm2: runs as the SSH user, no sudo",
            );
        }
        self.start(&mut y);
        y.line(
            2,
            &format!("env_file: {ENV_FILE}"),
            "the app's environment: fill in the .env next to this file",
        );
        y.line(
            2,
            "# shared: [uploads]",
            "folders kept across releases, in shared/",
        );
        y.line(
            2,
            "# keep: { open: 5, compressed: 15 }",
            "old releases kept for rollback",
        );
        if pm2 {
            y.line(2, "# pm2:", "");
            y.line(2, "#   options:", "PM2's own app properties:");
            y.line(
                2,
                "#     max_memory_restart: 300M",
                "https://pm2.keymetrics.io/docs/usage/application-declaration/",
            );
        }
        y.raw("");
        y.raw("pipeline:");
        self.pipeline(&mut y);
        y.raw("  # Waits for the service to be active; with a url, also until it answers 2xx.");
        y.raw("  # health:");
        y.raw(&format!(
            "  #   url: http://127.0.0.1:{}/health",
            self.port()
        ));
        y.line(
            2,
            "#   rollback: ask",
            "or auto: roll back at once when it fails",
        );
        y.0
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
        format!("{what}: {}.", self.preset.summary())
    }

    /// A port for the example health url.
    fn port(&self) -> u16 {
        match self.preset {
            Preset::Node | Preset::Bun => 3000,
            Preset::Python => 8000,
            _ => 8080,
        }
    }

    /// `start` and `env` under `service`.
    fn start(&self, y: &mut Yaml) {
        let name = &self.name;
        match self.preset {
            Preset::Node => {
                y.line(
                    2,
                    "start: node dist/main.js",
                    "run in the release; node is looked up on the server",
                );
                y.line(
                    2,
                    "env: { NODE_ENV: production }",
                    "set for the app and server commands",
                );
            }
            Preset::Java | Preset::Kotlin => y.line(
                2,
                &format!("start: build/install/{name}/bin/{name}"),
                "the script installDist writes; it runs java",
            ),
            Preset::Python => y.line(
                2,
                "start: .venv/bin/python main.py",
                "run in the release; point it at your entry point",
            ),
            Preset::Go | Preset::Rust | Preset::Dart | Preset::Bun => y.line(
                2,
                &format!("# start: ./{name} --port {}", self.port()),
                "default: the binary below, with no arguments",
            ),
        }
    }

    fn pipeline(&self, y: &mut Yaml) {
        let name = &self.name;
        let requires = match self.preset {
            Preset::Node => Some("npm"),
            Preset::Java | Preset::Kotlin => Some("java"),
            Preset::Python => Some("python3"),
            _ => None,
        };
        if let Some(tool) = requires {
            y.raw("  preflight:");
            y.line(
                4,
                &format!("requires: [{tool}]"),
                "the server must have these; checked before the build",
            );
        }
        let build = match self.preset {
            Preset::Node => Some("[npm ci, npm run build]".to_string()),
            Preset::Go => Some(format!(
                "CGO_ENABLED=0 GOOS=linux go build -o dist/{name} ."
            )),
            Preset::Rust => Some("cargo build --release".to_string()),
            Preset::Dart => Some(format!(
                "[dart pub get, dart compile exe bin/server.dart -o dist/{name}]"
            )),
            Preset::Bun => Some(format!(
                "[bun install, bun build ./src/index.ts --compile --target=bun-linux-x64 --outfile dist/{name}]"
            )),
            Preset::Java | Preset::Kotlin => Some("./gradlew installDist".to_string()),
            Preset::Python => None,
        };
        if let Some(build) = build {
            y.raw("  build:");
            y.raw("    run:");
            y.raw("      # Runs here, in project; $SHIPDECK_VERSION holds the release's version.");
            y.raw(&format!("      local: {build}"));
        }
        y.raw("  pack:");
        let binary = match self.preset {
            Preset::Go | Preset::Dart | Preset::Bun => Some(format!("dist/{name}")),
            Preset::Rust => Some(format!("target/release/{name}")),
            _ => None,
        };
        if let Some(binary) = binary {
            y.line(
                4,
                &format!("binary: {binary}"),
                "what the build makes; checked against the server's CPU",
            );
        }
        y.raw("    files:");
        y.raw("      include:");
        let project = match self.preset {
            Preset::Node => Some("[dist, package.json, package-lock.json]".to_string()),
            Preset::Java | Preset::Kotlin => Some(format!("[build/install/{name}]")),
            Preset::Python => Some("[.]".to_string()),
            _ => None,
        };
        match project {
            Some(list) => y.line(
                8,
                &format!("project: {list}"),
                "from project, at the same paths",
            ),
            None => y.line(
                8,
                "# project: [templates]",
                "files from project, at the same paths",
            ),
        }
        y.line(
            8,
            &format!("deploy: [{ENV_FILE}]"),
            "files next to this deploy.yaml",
        );
        if self.preset == Preset::Python {
            y.raw("      exclude:");
            y.line(
                8,
                "project: [.venv, \"**/__pycache__\", \"**/*.pyc\"]",
                "left out of include",
            );
        }
        let setup: &[&str] = match self.preset {
            Preset::Node => &["npm ci --omit=dev"],
            Preset::Python => &[
                "python3 -m venv .venv",
                ".venv/bin/pip install -q -r requirements.txt",
            ],
            _ => &[],
        };
        if !setup.is_empty() {
            y.raw("  setup:");
            y.raw("    run:");
            y.raw("      # Runs on the server in the new release, before it goes live.");
            y.raw("      server:");
            for command in setup {
                y.raw(&format!("        - {command}"));
            }
        }
    }
}

/// Lines of YAML, with comments lined up after the values.
#[derive(Default)]
struct Yaml(String);

impl Yaml {
    /// The column comments start at, when the value leaves room.
    const COMMENT_AT: usize = 36;

    fn raw(&mut self, line: &str) {
        self.0.push_str(line);
        self.0.push('\n');
    }

    fn line(&mut self, indent: usize, text: &str, comment: &str) {
        let text = format!("{:indent$}{text}", "");
        if comment.is_empty() {
            return self.raw(&text);
        }
        let width = Self::COMMENT_AT.max(text.len() + 2);
        self.raw(&format!("{text:<width$}# {comment}"));
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
