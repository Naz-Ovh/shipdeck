//! The three forms a path on your PC can take, and the paths a release holds.

use std::path::{Component, Path, PathBuf};

/// How a path in a deploy file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// `/opt/x`: absolute.
    Absolute,
    /// `./x` or `../x`: from the deploy file's folder.
    Config,
    /// `apps/x`: from the project folder.
    Project,
}

/// The form `text` is written in.
#[must_use]
pub fn form(text: &str) -> Form {
    let path = Path::new(text);
    if text.starts_with('/')
        || text.starts_with('\\')
        || path.is_absolute()
        || matches!(path.components().next(), Some(Component::Prefix(_)))
    {
        return Form::Absolute;
    }
    if text == "." {
        return Form::Project;
    }
    if text == ".."
        || text.starts_with("./")
        || text.starts_with("../")
        || text.starts_with(".\\")
        || text.starts_with("..\\")
    {
        return Form::Config;
    }
    Form::Project
}

/// Where `text` is on your PC.
#[must_use]
pub fn local(project: &Path, config: &Path, text: &str) -> PathBuf {
    match form(text) {
        Form::Absolute => PathBuf::from(text),
        Form::Config => config.join(text),
        Form::Project => project.join(text),
    }
}

/// Normalizes `.` and `..` without touching the filesystem.
#[must_use]
pub fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// `path` from `base`, both lexical paths, when it is inside it. An empty
/// result means `path` is `base`.
#[must_use]
pub fn relative(base: &Path, path: &Path) -> Option<String> {
    let base = lexical(base);
    let path = lexical(path);
    let rest = path.strip_prefix(&base).ok()?;
    Some(slash(rest))
}

/// A `/`-separated path, without a leading `./`.
#[must_use]
pub fn slash(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

/// A path in the release: relative, `/`-separated, with no empty part.
///
/// # Errors
/// Returns why it cannot be a path in the release.
pub fn release_path(text: &str) -> Result<String, String> {
    if text.contains(['\\', '\n', '\r']) {
        return Err("must not contain backslashes or newlines".into());
    }
    if text.starts_with('/') {
        return Err("must not start with `/`".into());
    }
    let mut parts = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err("must not climb out with `..`".into()),
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err("must name something inside the release".into());
    }
    Ok(parts.join("/"))
}

/// A path joining `folder` and `path`, both release paths.
#[must_use]
pub fn join(folder: &str, path: &str) -> String {
    if folder.is_empty() {
        path.to_string()
    } else if path.is_empty() {
        folder.to_string()
    } else {
        format!("{folder}/{path}")
    }
}

/// Whether `folder` and `path` are the same or one is inside the other.
#[must_use]
pub fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (Path::new(a), Path::new(b));
    a.starts_with(b) || b.starts_with(a)
}

/// Whether `text` holds a glob wildcard.
#[must_use]
pub fn has_wildcard(text: &str) -> bool {
    text.contains(['*', '?', '[', '{', '\\'])
}

#[cfg(test)]
#[path = "paths_tests.rs"]
mod tests;
