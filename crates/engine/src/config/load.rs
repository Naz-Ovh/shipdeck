//! Reading the project file and the deploy files it names.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::names::is_plain_name;
use super::paths;
use super::problem::{FileProblems, Problem};
use super::resolve::Deployment;
use super::rules;
use super::types::{DeployFile, ProjectFile};
use super::{check, schema, yaml};
use crate::ssh::Endpoint;

/// The project file names, in the order the message names them.
pub const PROJECT_NAMES: [&str; 2] = ["shipdeck.yaml", ".shipdeck"];

/// The project file of the folder shipdeck runs in.
#[derive(Debug, Clone)]
pub struct Project {
    /// The folder shipdeck runs in.
    pub folder: PathBuf,
    /// The project file.
    pub file: PathBuf,
    /// The project file's text.
    pub yaml: String,
    /// Its deploys: name, and the deploy file of each.
    pub deploys: BTreeMap<String, PathBuf>,
}

impl Project {
    /// Reads the project file in `folder`, or says the file it expected.
    ///
    /// # Errors
    /// Returns the problems when there is no project file, both names exist,
    /// or the file cannot be used.
    pub fn read(folder: &Path) -> Result<Self, FileProblems> {
        let found: Vec<PathBuf> = PROJECT_NAMES
            .iter()
            .map(|name| folder.join(name))
            .filter(|path| path.is_file())
            .collect();
        let path = match found.as_slice() {
            [one] => one.clone(),
            [] => {
                return Err(FileProblems::without_source(
                    folder.join(PROJECT_NAMES[0]),
                    Problem::new(format!(
                        "{} has no project file; expected {} or {}",
                        folder.display(),
                        PROJECT_NAMES[0],
                        PROJECT_NAMES[1]
                    ))
                    .help("run shipdeck in the folder that holds it, or name a deploy file with --shipdeckfile"),
                ));
            }
            _ => {
                return Err(FileProblems::without_source(
                    found[0].clone(),
                    Problem::new(format!(
                        "this folder has both {} and {}",
                        PROJECT_NAMES[0], PROJECT_NAMES[1]
                    ))
                    .help("keep one of them"),
                ));
            }
        };
        let text = read(&path)?;
        let file: ProjectFile =
            parse_document(&text, &schema::project()).map_err(|problems| FileProblems {
                path: path.clone(),
                source: Some(text.clone()),
                problems,
            })?;
        let mut problems = Vec::new();
        let mut deploys = BTreeMap::new();
        for (name, written) in &file.deploys {
            if !is_plain_name(name) {
                problems.push(Problem::new(format!(
                    "deploy name {name:?} is not a plain name"
                )));
                continue;
            }
            let file = paths::local(folder, folder, written);
            deploys.insert(name.clone(), file);
        }
        if !problems.is_empty() {
            return Err(FileProblems {
                path,
                source: Some(text),
                problems,
            });
        }
        Ok(Self {
            folder: folder.to_path_buf(),
            file: path,
            yaml: text,
            deploys,
        })
    }

    /// The deploy file of a deploy, when the project lists it.
    #[must_use]
    pub fn deploy_file(&self, name: &str) -> Option<&Path> {
        self.deploys.get(name).map(PathBuf::as_path)
    }
}

/// Reads and checks a deploy file.
///
/// # Errors
/// Returns every problem in the file, each with its line.
pub fn load_deploy(
    project: &Path,
    file: &Path,
    slug: Option<&str>,
) -> Result<Deployment, FileProblems> {
    let text = read(file)?;
    let deploy = parse_deploy(&text).map_err(|problems| FileProblems {
        path: file.to_path_buf(),
        source: Some(text.clone()),
        problems,
    })?;
    let config_dir = file
        .parent()
        .map(Path::to_path_buf)
        .map_or_else(|| project.to_path_buf(), |path| path.clone());
    let endpoint = Endpoint::resolve(&deploy.server.ssh).map_err(|err| FileProblems {
        path: file.to_path_buf(),
        source: Some(text.clone()),
        problems: vec![Problem::new(format!("{err:#}")).label("cannot log in like this")],
    })?;
    let slug = slug.map_or_else(|| deploy.name.clone(), str::to_string);
    Ok(Deployment {
        slug,
        file: file.to_path_buf(),
        project: project.to_path_buf(),
        config_dir,
        yaml: text,
        endpoint,
        deploy,
    })
}

/// Reads a deploy file's text, or says why not.
///
/// # Errors
/// Returns the read error as a problem.
pub fn read(path: &Path) -> Result<String, FileProblems> {
    std::fs::read_to_string(path).map_err(|err| {
        FileProblems::without_source(
            path.to_path_buf(),
            Problem::new(format!("cannot read it: {err}")),
        )
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
