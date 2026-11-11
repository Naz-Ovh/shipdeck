//! Reading the root file and the deploy files it names.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::names::is_deploy_name;
use super::paths;
use super::problem::{FileProblems, Problem};
use super::resolve::Deployment;
use super::rules;
use super::server_inputs::{Details, Failure, ServerInputs};
use super::types::{DeployEntry, DeployFile, RootFile};
use super::{check, schema, yaml};
use crate::ssh::Endpoint;

/// The root file names, in the order the message names them.
pub const ROOT_NAMES: [&str; 2] = ["shipdeck.yaml", ".shipdeck"];

/// The root file of the project folder.
#[derive(Debug, Clone)]
pub struct Root {
    /// The project folder.
    pub folder: PathBuf,
    /// The root file.
    pub file: PathBuf,
    /// The root file's text.
    pub yaml: String,
    /// Its deploys: name, and the deploy file of each or the deploy itself.
    pub deploys: BTreeMap<String, Deploy>,
}

/// One deploy of a root file: a file on your PC, or the deploy written in
/// the root file.
#[derive(Debug, Clone)]
pub enum Deploy {
    /// A deploy file, at its full path.
    File(PathBuf),
    /// A deploy written in the root file.
    InPlace(Box<DeployFile>),
}

impl Root {
    /// Reads the root file in `folder`, or the file `named`, or says the
    /// names it expected.
    ///
    /// # Errors
    /// Returns the problems when there is no root file, both names exist, or
    /// the file cannot be used.
    pub fn read(folder: &Path, named: Option<&Path>) -> Result<Self, FileProblems> {
        let path = match named {
            Some(path) => path.to_path_buf(),
            None => find(folder)?,
        };
        if !path.is_file() {
            return Err(FileProblems::without_source(
                path.clone(),
                Problem::new(format!("{} does not exist", path.display())),
            ));
        }
        let text = read(&path)?;
        let problems = |found: Vec<Problem>| FileProblems {
            path: path.clone(),
            source: Some(text.clone()),
            problems: found,
        };
        let node = yaml::parse(&text).map_err(|problem| problems(vec![problem]))?;
        // A deploy file named by `-c` is a mistake worth naming precisely.
        if node.find(&["deploys"]).is_none() && node.find(&["name"]).is_some() {
            let shown = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            return Err(problems(vec![
                Problem::new(format!(
                    "{} is a deploy file, not a root file",
                    path.display()
                ))
                .help(format!("run `shipdeck deploy {shown}`")),
            ]));
        }
        let file: RootFile = parse_document(&text, &schema::root_file()).map_err(problems)?;
        if !file.root {
            return Err(problems(vec![
                Problem::new(format!("{} is not a root file", path.display()))
                    .at(node.span_of(&["root"]))
                    .label("`root` is false")
                    .help("a root file needs `root: true`"),
            ]));
        }
        let from = path
            .parent()
            .map_or_else(|| folder.to_path_buf(), Path::to_path_buf);
        let mut found = Vec::new();
        let mut deploys = BTreeMap::new();
        for (name, entry) in &file.deploys {
            let at = &["deploys", name.as_str()];
            if !is_deploy_name(name) {
                found.push(
                    Problem::new(format!("deploy name {name:?} is not a deploy name"))
                        .at(node.key_span(at))
                        .label("not allowed")
                        .help("letters, digits, `-` and `_`"),
                );
                continue;
            }
            match entry {
                DeployEntry::File(written) => {
                    if !is_deploy_file(written) {
                        found.push(
                            Problem::new(format!("{written:?} does not end in .yaml or .yml"))
                                .at(node.span_of(at))
                                .label("not a deploy file")
                                .help("a deploy file's name ends in .yaml or .yml"),
                        );
                        continue;
                    }
                    let file = paths::local(&from, &from, written);
                    if !file.is_file() {
                        found.push(
                            Problem::new(format!("{} does not exist", file.display()))
                                .at(node.span_of(at))
                                .label("no such file"),
                        );
                        continue;
                    }
                    deploys.insert(name.clone(), Deploy::File(file));
                }
                DeployEntry::InPlace(deploy) => {
                    found.extend(check_in_place(deploy, &node, at));
                    deploys.insert(name.clone(), Deploy::InPlace(deploy.clone()));
                }
            }
        }
        if !found.is_empty() {
            return Err(problems(found));
        }
        Ok(Self {
            folder: folder.to_path_buf(),
            file: path,
            yaml: text,
            deploys,
        })
    }

    /// The deployment of a deploy the root file lists.
    ///
    /// # Errors
    /// Returns the problems when the deploy or its login cannot be used.
    pub fn deployment(&self, name: &str, details: Details<'_>) -> Result<Deployment, FileProblems> {
        match self.deploys.get(name) {
            Some(Deploy::File(file)) => load_deploy(&self.folder, file, Some(name), details),
            Some(Deploy::InPlace(deploy)) => load_in_place(self, name, deploy, details),
            None => Err(FileProblems::without_source(
                self.file.clone(),
                Problem::new(format!(
                    "{} has no deploy named {name}",
                    self.file.display()
                )),
            )),
        }
    }
}

/// The root file of `folder`: its one name, or the error that names both.
fn find(folder: &Path) -> Result<PathBuf, FileProblems> {
    let found: Vec<PathBuf> = ROOT_NAMES
        .iter()
        .map(|name| folder.join(name))
        .filter(|path| path.is_file())
        .collect();
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(FileProblems::without_source(
            folder.join(ROOT_NAMES[0]),
            Problem::new(format!("there is no root file in {}", folder.display()))
                .label(format!("expected {} or {}", ROOT_NAMES[0], ROOT_NAMES[1]))
                .help(
                    "run shipdeck in the folder that holds one, name it with --config, \
                     or name a deploy file ending in .yaml or .yml",
                ),
        )),
        _ => Err(FileProblems::without_source(
            found[0].clone(),
            Problem::new(format!(
                "this folder has both {} and {}",
                ROOT_NAMES[0], ROOT_NAMES[1]
            ))
            .help("keep one of them"),
        )),
    }
}

/// Every deploy of a run: the named deploy files, or the root file's deploys.
///
/// Returns the deploys and the problems, so one bad file does not hide the
/// rest. A deploy that fails only on a variable that is not set is listed as
/// unavailable, unless the run names it: then it is a problem. The flags and
/// `SHIPDECK_SSH_*` variables in `inputs` apply to the run's one deploy, and
/// are refused for more.
#[must_use]
pub fn open_all(
    folder: &Path,
    config: Option<&Path>,
    deploy_files: &[PathBuf],
    inputs: &ServerInputs,
) -> (Vec<Deployment>, Vec<FileProblems>) {
    let mut deploys = Vec::new();
    let mut problems = Vec::new();
    if deploy_files.is_empty() {
        let root = match Root::read(folder, config) {
            Ok(root) => root,
            Err(found) => return (deploys, vec![found]),
        };
        let run = inputs.run(root.deploys.keys());
        let sole = match inputs.single(run.len()) {
            Ok(sole) => sole,
            Err(message) => return (deploys, vec![refused(&root.file, message)]),
        };
        for name in root.deploys.keys() {
            let overridden = sole && run.first() == Some(&name.as_str());
            // A deploy the run does not use is listed as unavailable, not refused.
            let used = inputs.names_run() && run.contains(&name.as_str());
            let details = inputs.details(overridden);
            let details = if used {
                details
            } else {
                details.or_unavailable()
            };
            match root.deployment(name, details) {
                Ok(deployment) => deploys.push(deployment),
                Err(found) => problems.push(found),
            }
        }
        return (deploys, problems);
    }
    let sole = match inputs.single(deploy_files.len()) {
        Ok(sole) => sole,
        Err(message) => return (deploys, vec![refused(&deploy_files[0], message)]),
    };
    for (index, file) in deploy_files.iter().enumerate() {
        let details = inputs.details(sole && index == 0);
        match load_deploy(folder, file, None, details) {
            Ok(deployment) => deploys.push(deployment),
            Err(found) => problems.push(found),
        }
    }
    (deploys, problems)
}

/// A run that the flags and variables cannot serve, as a problem of `file`.
fn refused(file: &Path, message: String) -> FileProblems {
    FileProblems::without_source(file.to_path_buf(), Problem::new(message))
}

/// The problems of a deploy written in the root file, at its lines there.
fn check_in_place(deploy: &DeployFile, root: &yaml::Node, at: &[&str]) -> Vec<Problem> {
    let Some(node) = root.find(at) else {
        return Vec::new();
    };
    rules::check(deploy, Some(node))
}

/// Reads and checks a deploy file at its full path.
///
/// # Errors
/// Returns every problem in the file, each with its line.
pub fn load_deploy(
    project: &Path,
    file: &Path,
    slug: Option<&str>,
    details: Details<'_>,
) -> Result<Deployment, FileProblems> {
    let text = read(file)?;
    let mut deploy = match parse_deploy(&text) {
        Ok(deploy) => deploy,
        Err(problems) => {
            return Err(FileProblems {
                path: file.to_path_buf(),
                source: Some(text),
                problems: root_in_deploy(file, problems),
            });
        }
    };
    let config_dir = file
        .parent()
        .map_or_else(|| project.to_path_buf(), Path::to_path_buf);
    let slug = slug.map_or_else(|| deploy.name.clone(), str::to_string);
    let settled = settle_server(&mut deploy, &slug, details, &text, &[]).map_err(|problems| {
        FileProblems {
            path: file.to_path_buf(),
            source: Some(text.clone()),
            problems,
        }
    })?;
    Ok(Deployment {
        slug,
        file: file.to_path_buf(),
        project: project.to_path_buf(),
        config_dir,
        yaml: text,
        endpoint: settled.endpoint,
        ssh_from: settled.from,
        unavailable: settled.unavailable,
        deploy,
    })
}

/// A deploy's server, decided.
struct Settled {
    endpoint: Endpoint,
    /// Details that did not come from the file.
    from: Vec<String>,
    /// Why the deploy cannot run, when a variable it names is not set.
    unavailable: Option<String>,
}

/// Decides the deploy's server details, writes the final ones into `deploy`,
/// and applies `~/.ssh/config`. `at` is where the deploy sits in `text`.
///
/// With `details.or_unavailable()`, a deploy that fails only because a
/// variable is not set is settled as unavailable, not as an error.
fn settle_server(
    deploy: &mut DeployFile,
    slug: &str,
    details: Details<'_>,
    text: &str,
    at: &[&str],
) -> Result<Settled, Vec<Problem>> {
    let applied = match details.apply(&deploy.server.ssh, slug) {
        Ok(applied) => applied,
        Err(failures) if details.is_lenient() && failures.iter().all(|failure| failure.unset) => {
            let reason = failures
                .iter()
                .map(|failure| format!("{}. {}", failure.message, failure.help))
                .collect::<Vec<_>>()
                .join(" ");
            return Ok(Settled {
                endpoint: Endpoint::placeholder(&deploy.server.ssh),
                from: Vec::new(),
                unavailable: Some(reason),
            });
        }
        Err(failures) => return Err(problems_at(failures, text, at)),
    };
    deploy.server.ssh = applied.ssh;
    let endpoint = Endpoint::resolve(&deploy.server.ssh)
        .map_err(|err| vec![Problem::new(format!("{err:#}")).label("cannot log in like this")])?;
    Ok(Settled {
        endpoint,
        from: applied.from,
        unavailable: None,
    })
}

/// The failures as problems, each at its field's line in `text`.
fn problems_at(failures: Vec<Failure>, text: &str, at: &[&str]) -> Vec<Problem> {
    let node = yaml::parse(text).ok();
    failures
        .into_iter()
        .map(|failure| {
            let mut path = at.to_vec();
            path.extend(["server", "ssh", failure.field]);
            Problem::new(failure.message)
                .at(node.as_ref().and_then(|node| node.span_of(&path)))
                .label("cannot be used")
                .help(failure.help)
        })
        .collect()
}

/// The problems of a deploy file that holds `root`, named as the mistake it is.
fn root_in_deploy(file: &Path, problems: Vec<Problem>) -> Vec<Problem> {
    let holds_root = problems
        .iter()
        .any(|problem| problem.message.contains("`root`"));
    if !holds_root {
        return problems;
    }
    vec![
        Problem::new(format!(
            "{} is a root file, not a deploy file",
            file.display()
        ))
        .help(format!("name it with `-c {}`", file.display())),
    ]
}

/// A deploy written in the root file, as a deployment.
///
/// Its `./x` paths and `{{CONFIG}}` start from the root file's folder.
///
/// # Errors
/// Returns the problems when the deploy cannot log in like this.
pub fn load_in_place(
    root: &Root,
    name: &str,
    deploy: &DeployFile,
    details: Details<'_>,
) -> Result<Deployment, FileProblems> {
    let project = &root.folder;
    let config_dir = root
        .file
        .parent()
        .map_or_else(|| project.clone(), Path::to_path_buf);
    let mut deploy = deploy.clone();
    let settled = settle_server(&mut deploy, name, details, &root.yaml, &["deploys", name])
        .map_err(|problems| FileProblems {
            path: root.file.clone(),
            source: Some(root.yaml.clone()),
            problems,
        })?;
    Ok(Deployment {
        slug: name.to_string(),
        file: root.file.clone(),
        project: project.clone(),
        config_dir,
        yaml: root.yaml.clone(),
        endpoint: settled.endpoint,
        ssh_from: settled.from,
        unavailable: settled.unavailable,
        deploy,
    })
}

/// Reads a file's text, or says why not, with its full path.
///
/// # Errors
/// Returns the read error as a problem.
pub fn read(path: &Path) -> Result<String, FileProblems> {
    std::fs::read_to_string(path).map_err(|err| {
        FileProblems::without_source(
            path.to_path_buf(),
            Problem::new(format!("cannot read {}: {err}", full(path).display())),
        )
    })
}

/// The full path of `path`, for a message about a file on this PC.
#[must_use]
pub fn full(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether a file is written as a deploy file: its name ends in `.yaml` or
/// `.yml`. Only the ending decides.
#[must_use]
pub fn is_deploy_file(text: &str) -> bool {
    Path::new(text).extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
    })
}

/// Reads a deploy file's text and checks it.
///
/// # Errors
/// Returns every problem in the text, each with its line.
pub fn parse_deploy(text: &str) -> Result<DeployFile, Vec<Problem>> {
    let root = yaml::parse(text).map_err(|problem| vec![problem])?;
    let schema = schema::deploy();
    let checked = check::structure_with(&root, &schema);
    let mut problems = checked.problems;
    match serde_json::from_value::<DeployFile>(checked.value) {
        Ok(deploy) => {
            problems.extend(rules::check(&deploy, Some(&root)));
            if problems.is_empty() {
                return Ok(deploy);
            }
        }
        Err(err) if problems.is_empty() => {
            problems.push(Problem::new(format!(
                "this deploy file could not be read: {err}"
            )));
        }
        Err(_) => {}
    }
    problems.sort_by_key(|problem| problem.at.map(|at| at.start));
    Err(problems)
}

/// Reads one YAML document against a schema type.
fn parse_document<T: DeserializeOwned>(text: &str, schema: &Value) -> Result<T, Vec<Problem>> {
    let root = yaml::parse(text).map_err(|problem| vec![problem])?;
    let checked = check::structure_with(&root, schema);
    let mut problems = checked.problems;
    match serde_json::from_value::<T>(checked.value) {
        Ok(value) if problems.is_empty() => Ok(value),
        Ok(_) => Err(problems),
        Err(err) => {
            if problems.is_empty() {
                problems.push(Problem::new(format!("this file could not be read: {err}")));
            }
            problems.sort_by_key(|problem| problem.at.map(|at| at.start));
            Err(problems)
        }
    }
}

#[cfg(test)]
#[path = "load_tests.rs"]
mod tests;
