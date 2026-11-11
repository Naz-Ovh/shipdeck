//! `shipdeck init`: a root file and a deploy file, written from a preset.
//! What it writes goes through the same checks as a launch.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{DeployFile, FileProblems, ManagerKind, parse_deploy};

/// The `.env` every preset ships; it starts empty and stays out of git.
const ENV_FILE: &str = ".env";
/// The deploy file's name, beside the root file.
const DEPLOY_FILE: &str = "deploy.yaml";
/// Where the README lists every field.
const DOCS: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "#the-deploy-file");

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
    #[must_use]
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

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }

    /// How the app is built and what reaches the server, in a few words.
    #[must_use]
    pub fn summary(self) -> &'static str {
        match self {
            Preset::Node => "built here; the server installs production dependencies",
            Preset::Go => "one static binary, nothing to install on the server",
            Preset::Java | Preset::Kotlin => "built with Gradle; the server needs Java",
            Preset::Rust => "one binary, nothing to install on the server",
            Preset::Dart | Preset::Bun => {
                "compiled to one executable, nothing to install on the server"
            }
            Preset::Python => "the source ships; the server builds its virtualenv",
        }
    }

    /// A port for the example health URL.
    fn port(self) -> u16 {
        match self {
            Preset::Node | Preset::Bun => 3000,
            Preset::Python => 8000,
            _ => 8080,
        }
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Everything `init` needs to write a deploy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDeployable {
    /// The deploy's name, also the default process name.
    pub name: String,
    pub preset: Preset,
    pub manager: ManagerKind,
    /// An address or an alias from `~/.ssh/config`.
    pub host: String,
    /// Left out to use the alias's `User`.
    pub user: Option<String>,
    /// The app's source folder, as the config file sees it.
    pub project: String,
}

impl NewDeployable {
    /// The deploy file's text.
    #[must_use]
    pub fn yaml(&self) -> String {
        let mut y = Yaml::default();
        y.raw(&crate::config::schema::modeline());
        y.raw(&format!("# {}", self.comment()));
        y.raw(&format!("# Every field: {DOCS}"));
        y.raw("");
        self.name_and_server(&mut y);
        self.release(&mut y);
        self.app(&mut y);
        y.0
    }

    /// `name`, `server` and the blank line after them.
    fn name_and_server(&self, y: &mut Yaml) {
        y.line(
            0,
            &format!("name: {}", quote(&self.name)),
            "the deploy's name; process names start with it",
        );
        y.raw("");
        y.raw("server:");
        y.raw("  ssh:");
        y.line(
            4,
            &format!("host: {}", quote(&self.host)),
            "an address, or an alias from ~/.ssh/config",
        );
        match &self.user {
            Some(user) => y.line(
                4,
                &format!("user: {}", quote(user)),
                "logs in and deploys as this user",
            ),
            None => y.line(4, "# user: deploy", "default: the alias's User"),
        }
        y.line(
            2,
            &format!("dir: {}", quote(&format!("/opt/{}", self.name))),
            "current/, releases/, backups/, shared/",
        );
        y.raw("");
    }

    /// `release:`: the requires, the build and the rollback mode.
    fn release(&self, y: &mut Yaml) {
        y.raw("release:");
        y.raw("  steps:");
        if let Some(tool) = self.requires() {
            y.raw("    preflight:");
            y.line(
                6,
                &format!("requires: [{tool}]"),
                "the server must have these",
            );
        }
        if let Some(build) = self.build() {
            y.raw("    build:");
            y.line(
                6,
                &format!("local: {build}"),
                "runs here, before the files are collected",
            );
        }
        y.raw("    rollback:");
        y.line(6, "mode: ask", "ask | auto | off");
        y.raw("");
    }

    /// `apps:`: one app with its source, files, process and steps.
    fn app(&self, y: &mut Yaml) {
        y.raw("apps:");
        y.raw(&format!("  {}:", self.name));
        y.line(
            4,
            &format!("source: {}", quote(&self.project)),
            "your source, on your PC",
        );
        let files = self.files();
        if !files.is_empty() {
            y.line(
                4,
                &format!("files: [{}]", files.join(", ")),
                "what reaches the server",
            );
        }
        if let Some(binary) = self.binary() {
            y.line(
                4,
                &format!("binary: {binary}"),
                "lands in the app folder with mode 0755",
            );
        }
        y.raw("    process:");
        if let Some(start) = self.start() {
            y.line(
                6,
                &format!("start: {}", quote(&start)),
                "runs in the app folder",
            );
        }
        if let Some(env) = self.env() {
            y.line(
                6,
                &format!("env: {{ {env} }}"),
                "set for the app and its commands",
            );
        }
        match self.manager {
            ManagerKind::Systemd => y.raw("      systemd: {}"),
            ManagerKind::Pm2 => y.raw("      pm2: {}"),
            ManagerKind::Oxmgr => y.raw("      oxmgr: {}"),
        }
        y.raw("    steps:");
        if let Some(setup) = self.setup() {
            y.raw("      setup:");
            y.line(
                8,
                &format!("server: {}", quote(&setup)),
                "runs on the server in the new release",
            );
        }
        y.raw("      health:");
        y.line(
            8,
            &format!("url: http://127.0.0.1:{}/", self.preset.port()),
            "must answer 2xx before the release is live",
        );
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

    fn requires(&self) -> Option<&'static str> {
        match self.preset {
            Preset::Node => Some("npm"),
            Preset::Java | Preset::Kotlin => Some("java"),
            Preset::Python => Some("python3"),
            _ => None,
        }
    }

    fn build(&self) -> Option<String> {
        let name = &self.name;
        match self.preset {
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
        }
    }

    fn files(&self) -> Vec<String> {
        let name = &self.name;
        let mut files: Vec<String> = match self.preset {
            Preset::Node => vec![
                "dist".into(),
                "package.json".into(),
                "package-lock.json".into(),
            ],
            Preset::Java | Preset::Kotlin => vec![format!("build/install/{name}")],
            Preset::Python => vec![".".into()],
            _ => Vec::new(),
        };
        files.push(ENV_FILE.into());
        files
    }

    fn binary(&self) -> Option<String> {
        match self.preset {
            Preset::Go | Preset::Dart | Preset::Bun => Some(format!("dist/{}", self.name)),
            Preset::Rust => Some(format!("target/release/{}", self.name)),
            _ => None,
        }
    }

    fn start(&self) -> Option<String> {
        let name = &self.name;
        match self.preset {
            Preset::Node => Some("node dist/main.js".into()),
            Preset::Java | Preset::Kotlin => Some(format!("build/install/{name}/bin/{name}")),
            Preset::Python => Some(".venv/bin/python main.py".into()),
            Preset::Go | Preset::Rust | Preset::Dart | Preset::Bun => None,
        }
    }

    fn env(&self) -> Option<&'static str> {
        match self.preset {
            Preset::Node => Some("NODE_ENV: production"),
            _ => None,
        }
    }

    fn setup(&self) -> Option<String> {
        match self.preset {
            Preset::Node => Some("npm ci --omit=dev".into()),
            Preset::Python => {
                Some("python3 -m venv .venv && .venv/bin/pip install -r requirements.txt".into())
            }
            _ => None,
        }
    }
}

/// Lines of YAML, with comments lined up after the values.
#[derive(Default)]
struct Yaml(String);

impl Yaml {
    /// The column comments start at, when the value leaves room.
    const COMMENT_AT: usize = 40;

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

/// A YAML double-quoted scalar. JSON's string syntax is a subset of it.
fn quote(text: &str) -> String {
    serde_json::Value::String(text.to_string()).to_string()
}

/// Why `name` cannot be a new deploy's folder, if it cannot.
#[must_use]
pub fn name_problem(root: &Path, name: &str) -> Option<String> {
    if !crate::config::is_plain_name(name) || name.starts_with('.') {
        return Some("use letters, digits, `.`, `_` and `-`, not starting with `.`".to_string());
    }
    let folder = root.join(name);
    if folder.exists() {
        return Some(format!("{} already exists", folder.display()));
    }
    None
}

/// `project` as the new deploy file sees it, from `root`.
///
/// # Errors
/// Returns an error when either path is missing or not a folder.
pub fn project_from(root: &Path, project: &Path) -> Result<String> {
    let root = root
        .canonicalize()
        .with_context(|| format!("{} is not a folder", root.display()))?;
    let project = project
        .canonicalize()
        .with_context(|| format!("the project folder {} does not exist", project.display()))?;
    if !project.is_dir() {
        bail!("{} is not a folder", project.display());
    }
    if root.components().next() != project.components().next() {
        bail!(
            "{} is on another drive than {}; a deploy file reaches its project by a relative path",
            project.display(),
            root.display()
        );
    }
    let parts: Vec<String> = relative(&root, &project)
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

/// The path from the folder `from` to `to`.
#[must_use]
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

/// The new deploy file's text, checked as a launch would check it.
///
/// # Errors
/// Returns every problem the generated config has.
pub fn check(root: &Path, new: &NewDeployable) -> Result<String, FileProblems> {
    let yaml = new.yaml();
    let file = root.join(DEPLOY_FILE);
    parse_deploy(&yaml).map_err(|problems| FileProblems {
        path: file.clone(),
        source: Some(yaml.clone()),
        problems,
    })?;
    Ok(yaml)
}

/// Writes the root file, the deploy file, an empty `.env` and a
/// `.gitignore`. Files already there are left alone.
///
/// # Errors
/// Returns an error when a config file is already there, or one cannot be
/// written.
pub fn write(root: &Path, name: &str, yaml: &str) -> Result<Vec<PathBuf>> {
    let root_file = root.join(crate::config::ROOT_NAMES[0]);
    if root_file.exists() {
        bail!("{} already exists", root_file.display());
    }
    let deploy = root.join(DEPLOY_FILE);
    let root_yaml = format!(
        "# The root file: `root: true`, and its deploys by name.\n# Every field: {DOCS}\n\nroot: true\ndeploys:\n  {}: {}\n",
        quote(name),
        DEPLOY_FILE
    );
    std::fs::write(&root_file, root_yaml)
        .with_context(|| format!("writing {}", root_file.display()))?;
    std::fs::write(&deploy, yaml).with_context(|| format!("writing {}", deploy.display()))?;
    let mut written = vec![root_file, deploy];
    for (file, text) in [(ENV_FILE, ""), (".gitignore", ".env\n")] {
        let path = root.join(file);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

/// The parsed deploy file, for callers that want it.
///
/// # Errors
/// Returns every problem in the text.
pub fn deploy_file(yaml: &str) -> Result<DeployFile, FileProblems> {
    parse_deploy(yaml).map_err(|problems| FileProblems {
        path: PathBuf::from(DEPLOY_FILE),
        source: Some(yaml.to_string()),
        problems,
    })
}

#[cfg(test)]
#[path = "init_tests.rs"]
mod tests;
