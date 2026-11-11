//! Every path the next release holds: files, links, and shipdeck's own
//! files in `.shipdeck/`. `deploy` and `shipdeck tree` both use this.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use super::pm2::pm2_file;
use super::unit::unit_file;
use crate::config::glob::{Rules, Verdict};
use crate::config::{Deployment, FileEntry, paths, release_path};
use crate::release::ReleaseName;

/// Where an entry comes from, for `shipdeck tree`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A release file.
    Release,
    /// A file of this app.
    App(String),
    /// A file shipdeck writes.
    Generated,
}

impl Origin {
    /// The tree's right-hand column.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Release => "release".into(),
            Self::App(name) => format!("app {name}"),
            Self::Generated => "generated".into(),
        }
    }
}

/// Where an entry's bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A file on your PC.
    File(PathBuf),
    /// Text shipdeck writes.
    Bytes(Vec<u8>),
    /// A link, kept as it is.
    Link(PathBuf),
}

/// One path of a release.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Relative to the release, `/`-separated.
    pub path: String,
    pub source: Source,
    pub mode: u32,
    pub size: u64,
    /// Filled in when the release is hashed.
    pub sha256: Option<String>,
    pub origin: Origin,
}

/// A shared folder, linked into the release on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Where the link is, relative to the release.
    pub path: String,
    /// The data it points at, absolute on the server.
    pub target: String,
}

/// Every path the next release holds.
#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub entries: Vec<Entry>,
    pub links: Vec<Link>,
    /// Include patterns that matched nothing.
    pub missing: Vec<String>,
    /// Folders a `files` entry named as a whole, which `tree --depth` folds.
    pub roots: BTreeSet<String>,
    /// SHA-256 of each generated unit or PM2 file as rendered for a neutral
    /// release, by path: what the fingerprint sees instead of the real file.
    pub neutral: BTreeMap<String, String>,
}

impl Plan {
    /// Sorts the entries by path, so pack, tree and the settings agree.
    pub fn sort(&mut self) {
        self.entries.sort_by(|a, b| a.path.cmp(&b.path));
        self.links.sort_by(|a, b| a.path.cmp(&b.path));
    }

    /// Every path that holds a file, in order.
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// The files the live release may already have: every regular file whose
    /// name `sha256sum` reads back.
    #[must_use]
    pub fn candidates(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| {
                matches!(entry.source, Source::File(_) | Source::Bytes(_))
                    && super::content::listable(&entry.path)
            })
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Replaces the bytes of one generated entry, after the manifest's
    /// fingerprint is known.
    ///
    /// # Errors
    /// Returns an error when the entry is not there.
    pub fn set_bytes(&mut self, path: &str, bytes: Vec<u8>) -> Result<()> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.path == path)
            .ok_or_else(|| anyhow!("{path} is not in the release"))?;
        entry.size = bytes.len() as u64;
        entry.sha256 = Some(super::content::sha256(&bytes));
        entry.source = Source::Bytes(bytes);
        Ok(())
    }
}

/// Builds the plan for one release.
///
/// # Errors
/// Returns an error for a file with no place, two files for one path, a
/// path shipdeck owns, or a file that cannot be read.
pub fn release_plan(
    deployment: &Deployment,
    release: &ReleaseName,
    manifest: &[u8],
    settings: &[u8],
    start_paths: &BTreeMap<String, String>,
) -> Result<Plan> {
    let mut plan = Plan::default();
    let links = shared_links(deployment);
    add_release_files(deployment, &mut plan)?;
    for name in deployment.deploy.apps.keys() {
        add_app_files(deployment, name, &mut plan)?;
        add_binary(deployment, name, &mut plan)?;
    }
    add_generated(
        deployment,
        release,
        manifest,
        settings,
        start_paths,
        &mut plan,
    )?;
    for link in links {
        if plan
            .entries
            .iter()
            .any(|entry| paths::overlaps(&link.path, &entry.path))
        {
            bail!(
                "{} is a shared folder, so nothing may be packed there",
                link.path
            );
        }
        plan.links.push(link);
    }
    check_parents(&plan)?;
    plan.sort();
    Ok(plan)
}

/// Every shared folder of the release and its apps.
#[must_use]
pub fn shared_links(deployment: &Deployment) -> Vec<Link> {
    let root = deployment.dir();
    let mut links = Vec::new();
    for entry in &deployment.deploy.release.shared {
        let Ok(path) = release_path(entry) else {
            continue;
        };
        links.push(Link {
            target: format!("{root}/shared/{path}"),
            path,
        });
    }
    for (name, app) in &deployment.deploy.apps {
        for entry in &app.shared {
            let Ok(folder) = release_path(entry) else {
                continue;
            };
            links.push(Link {
                path: format!("apps/{name}/{folder}"),
                target: format!("{root}/shared/apps/{name}/{folder}"),
            });
        }
    }
    links
}

fn add_release_files(deployment: &Deployment, plan: &mut Plan) -> Result<()> {
    let base = &deployment.project;
    let exclude = &deployment.deploy.release.exclude;
    for entry in &deployment.deploy.release.files {
        let (text, to) = match entry {
            FileEntry::Path(text) => (text.as_str(), None),
            FileEntry::To(mapped) => (mapped.path.as_str(), mapped.to.as_deref()),
        };
        let form = paths::form(text);
        if form == paths::Form::Project {
            let (found, missing) = walk_pattern(base, text, exclude)?;
            plan.missing.extend(missing);
            let literal = slash_literal(text);
            let root = folder_root(&deployment.project, text, to, &literal);
            if let Some(root) = root {
                plan.roots.insert(root);
            }
            for file in found {
                let path = match to {
                    None => file.path.clone(),
                    Some(to) => place(&file, &literal, to),
                };
                insert(plan, path, file, Origin::Release)?;
            }
        } else {
            let Some(to) = to else {
                bail!("{text} is outside the project folder, so it needs `to`");
            };
            let local = paths::local(&deployment.project, &deployment.config_dir, text);
            let found = walk_direct(&local)?;
            let root_is_dir = local.is_dir();
            if root_is_dir {
                let root = release_path(to).map_err(anyhow::Error::msg)?;
                plan.roots.insert(root);
            }
            for file in found {
                let path = if root_is_dir {
                    paths::join(to, &file.path)
                } else {
                    to.to_string()
                };
                insert(plan, path, file, Origin::Release)?;
            }
        }
    }
    Ok(())
}

fn add_app_files(deployment: &Deployment, name: &str, plan: &mut Plan) -> Result<()> {
    let app = &deployment.deploy.apps[name];
    let source = deployment.source(name);
    let folder = format!("apps/{name}");
    for entry in &app.files {
        let (text, to) = match entry {
            FileEntry::Path(text) => (text.as_str(), None),
            FileEntry::To(mapped) => (mapped.path.as_str(), mapped.to.as_deref()),
        };
        let form = paths::form(text);
        if form == paths::Form::Project {
            let (found, missing) = walk_pattern(&deployment.project, text, &app.exclude)?;
            plan.missing.extend(missing);
            let literal = slash_literal(text);
            if let Some(root) = folder_root(&deployment.project, text, to, &literal) {
                let destination = if to.is_some() {
                    paths::join(&folder, &root)
                } else {
                    let local = deployment.project.join(&literal);
                    match paths::relative(&source, &local) {
                        Some(relative) => paths::join(&folder, &relative),
                        None => paths::join(&folder, &root),
                    }
                };
                plan.roots.insert(destination);
            }
            for file in found {
                let destination = if let Some(to) = to {
                    paths::join(&folder, &place(&file, &literal, to))
                } else {
                    let local = deployment.project.join(&file.path);
                    let relative = paths::relative(&source, &local).with_context(|| {
                        format!(
                            "{} is outside the app's source {}",
                            file.path,
                            source.display()
                        )
                    })?;
                    paths::join(&folder, &relative)
                };
                insert(plan, destination, file, Origin::App(name.to_string()))?;
            }
        } else {
            let local = paths::local(&deployment.project, &deployment.config_dir, text);
            let found = walk_direct(&local)?;
            let root_is_dir = local.is_dir();
            if let Some(to) = to {
                if root_is_dir {
                    plan.roots.insert(paths::join(&folder, to));
                }
                for file in found {
                    let path = if root_is_dir {
                        paths::join(to, &file.path)
                    } else {
                        to.to_string()
                    };
                    insert(
                        plan,
                        paths::join(&folder, &path),
                        file,
                        Origin::App(name.to_string()),
                    )?;
                }
            } else {
                let relative = paths::relative(&source, &local).ok_or_else(|| {
                    anyhow::anyhow!("{text} is outside the app's source, so it needs `to`")
                })?;
                if root_is_dir {
                    plan.roots.insert(paths::join(&folder, &relative));
                }
                for file in found {
                    let path = if root_is_dir {
                        paths::join(&relative, &file.path)
                    } else {
                        relative.clone()
                    };
                    insert(
                        plan,
                        paths::join(&folder, &path),
                        file,
                        Origin::App(name.to_string()),
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn add_binary(deployment: &Deployment, name: &str, plan: &mut Plan) -> Result<()> {
    let Some(binary) = deployment.binary(name) else {
        return Ok(());
    };
    let written = match binary {
        FileEntry::Path(path) => path.as_str(),
        FileEntry::To(mapped) => mapped.path.as_str(),
    };
    let Some(local) = deployment.binary_local(name) else {
        return Ok(());
    };
    let Some(place) = deployment.binary_place(name) else {
        return Ok(());
    };
    let full = crate::config::full(&local);
    let metadata = std::fs::metadata(&local).with_context(|| {
        format!(
            "the build did not produce {written}, which is {}",
            full.display()
        )
    })?;
    if metadata.is_dir() {
        bail!("the binary {written} is a folder; `binary` names one file");
    }
    insert_entry(
        plan,
        &paths::join(&deployment.release_dir(name), &place),
        Entry {
            path: String::new(),
            source: Source::File(local),
            mode: 0o755,
            size: metadata.len(),
            sha256: None,
            origin: Origin::App(name.to_string()),
        },
    )
}

fn add_generated(
    deployment: &Deployment,
    release: &ReleaseName,
    manifest: &[u8],
    settings: &[u8],
    start_paths: &BTreeMap<String, String>,
    plan: &mut Plan,
) -> Result<()> {
    let bytes = |path: &str, text: Vec<u8>| Entry {
        path: path.to_string(),
        size: text.len() as u64,
        source: Source::Bytes(text),
        mode: 0o644,
        sha256: None,
        origin: Origin::Generated,
    };
    generated(
        plan,
        bytes(super::content::MANIFEST_FILE, manifest.to_vec()),
    )?;
    generated(plan, bytes(".shipdeck/settings.json", settings.to_vec()))?;
    // An nginx or oxmgr app stops at preflight, so the release never carries a
    // placeholder for them: `shipdeck tree` lists only files that will exist.
    for name in deployment.deploy.apps.keys() {
        let Some(manager) = deployment.manager(name) else {
            continue;
        };
        match manager {
            crate::config::Manager::Systemd(_) => {
                let path = unit_file(deployment, name);
                let start = start_paths.get(name).map(String::as_str);
                if let Some(text) = super::unit::render(deployment, name, release, start)? {
                    generated(plan, bytes(&path, text.into_bytes()))?;
                    if let Some(neutral) =
                        super::unit::render(deployment, name, &neutral_release(), start)?
                    {
                        plan.neutral
                            .insert(path, super::content::sha256(neutral.as_bytes()));
                    }
                }
            }
            crate::config::Manager::Pm2(_) => {
                if deployment.pm2_ecosystem(name).is_none() {
                    let path = pm2_file(deployment, name);
                    let text = super::pm2::render(deployment, name, release)?;
                    generated(plan, bytes(&path, text.into_bytes()))?;
                    let neutral = super::pm2::render(deployment, name, &neutral_release())?;
                    plan.neutral
                        .insert(path, super::content::sha256(neutral.as_bytes()));
                }
            }
            crate::config::Manager::Oxmgr(_) => {}
        }
    }
    Ok(())
}

/// The release name the fingerprint renders generated files with, so a new
/// release name alone never counts as a change.
fn neutral_release() -> ReleaseName {
    ReleaseName::new("0.0.0", "neutral", false, chrono::DateTime::UNIX_EPOCH)
}

fn generated(plan: &mut Plan, entry: Entry) -> Result<()> {
    if plan.entries.iter().any(|old| old.path == entry.path) {
        bail!("{} is in the release more than once", entry.path);
    }
    plan.entries.push(entry);
    Ok(())
}

fn insert(plan: &mut Plan, mut path: String, file: Found, origin: Origin) -> Result<()> {
    path = release_path(&path).map_err(anyhow::Error::msg)?;
    let source = match file.kind {
        Kind::File => Source::File(file.local.clone()),
        Kind::Link(target) => Source::Link(target),
    };
    insert_entry(
        plan,
        &path,
        Entry {
            path: String::new(),
            source,
            mode: file.mode,
            size: file.size,
            sha256: None,
            origin,
        },
    )
}

fn insert_entry(plan: &mut Plan, path: &str, mut entry: Entry) -> Result<()> {
    if path == ".shipdeck" || path.starts_with(".shipdeck/") {
        bail!("{path} belongs to shipdeck");
    }
    entry.path = path.to_string();
    if let Some(old) = plan.entries.iter().find(|old| old.path == path) {
        let same = old.source == entry.source && old.mode == entry.mode;
        if same {
            return Ok(());
        }
        return Err(anyhow::anyhow!(
            "{path} would hold two files: {} and {}",
            source_label(&old.source),
            source_label(&entry.source)
        ));
    }
    plan.entries.push(entry);
    Ok(())
}

fn source_label(source: &Source) -> String {
    match source {
        Source::File(path) => path.display().to_string(),
        Source::Bytes(_) => "a generated file".into(),
        Source::Link(target) => format!("a link to {}", target.display()),
    }
}

/// Refuses a release path whose parent is a file or a link.
fn check_parents(plan: &Plan) -> Result<()> {
    for entry in &plan.entries {
        let mut parent = Path::new(&entry.path).parent();
        while let Some(folder) = parent {
            if let Some(folder) = folder.to_str()
                && plan
                    .entries
                    .iter()
                    .find(|other| other.path == folder)
                    .is_some_and(|other| {
                        !matches!(other.source, Source::File(_) | Source::Bytes(_))
                    })
            {
                bail!("{} is inside a file or link {folder}", entry.path);
            }
            parent = folder.parent();
        }
    }
    Ok(())
}

/// Where a file picked by an entry with `to` lands.
fn place(file: &Found, literal: &str, to: &str) -> String {
    match strip_literal(&file.path, literal) {
        Some(rest) if !rest.is_empty() => paths::join(to, &rest),
        _ => to.to_string(),
    }
}

/// The part of `path` after `literal`, when it is inside it.
fn strip_literal(path: &str, literal: &str) -> Option<String> {
    if literal.is_empty() {
        return Some(path.to_string());
    }
    if path == literal {
        return Some(String::new());
    }
    path.strip_prefix(literal)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(str::to_string)
}

/// A `/`-separated literal path, without `./`.
fn slash_literal(text: &str) -> String {
    paths::slash(Path::new(text))
}

/// The release folder a whole-folder entry brings, when it names a folder.
fn folder_root(base: &Path, text: &str, to: Option<&str>, literal: &str) -> Option<String> {
    if paths::has_wildcard(text) {
        return None;
    }
    let local = base.join(text);
    if !local.is_dir() {
        return None;
    }
    match to {
        Some(to) => release_path(to).ok(),
        None => release_path(literal).ok(),
    }
}

/// One file the walk found.
#[derive(Debug, Clone)]
struct Found {
    /// Relative to the walk's base, `/`-separated.
    path: String,
    local: PathBuf,
    kind: Kind,
    mode: u32,
    size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    File,
    Link(PathBuf),
}

/// Walks `base` with a literal path or glob, leaving out `exclude`.
fn walk_pattern(base: &Path, text: &str, exclude: &[String]) -> Result<(Vec<Found>, Vec<String>)> {
    if !base.is_dir() {
        bail!("the project folder {} does not exist", base.display());
    }
    let rules = Rules::parse(&[text.to_string()], exclude)
        .map_err(|why| anyhow::anyhow!("{text:?}: {why}"))?;
    let mut walk = Walk {
        rules: &rules,
        hits: vec![0; rules.include.len()],
        items: Vec::new(),
    };
    let verdict = rules.judge(Verdict::default(), "", true, &mut walk.hits);
    walk.visit(base, "", verdict)?;
    let missing: Vec<String> = rules
        .include
        .iter()
        .zip(&walk.hits)
        .filter(|(pattern, hits)| !pattern.negated && **hits == 0)
        .map(|(pattern, _)| match pattern.literal() {
            Some(path) if !path.is_empty() => {
                format!("{} (not found)", paths::release_path(&path).unwrap_or(path))
            }
            Some(_) => text.to_string(),
            None => format!("{} (matched nothing)", pattern.text),
        })
        .collect();
    Ok((walk.items, missing))
}

struct Walk<'a> {
    rules: &'a Rules,
    hits: Vec<usize>,
    items: Vec<Found>,
}

impl Walk<'_> {
    fn visit(&mut self, dir: &Path, rel: &str, verdict: Verdict) -> Result<()> {
        let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        children.sort();
        for child in children {
            let name = child
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name == ".git" {
                continue;
            }
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            let mut meta = std::fs::symlink_metadata(&child)
                .with_context(|| format!("reading {}", child.display()))?;
            // A link the config names by its exact path is followed, so
            // `dist -> build/out` ships the folder.
            if meta.file_type().is_symlink()
                && self
                    .rules
                    .include
                    .iter()
                    .any(|pattern| pattern.literal().is_some_and(|literal| literal == path))
            {
                meta = std::fs::metadata(&child)
                    .with_context(|| format!("following the link {}", child.display()))?;
            }
            let is_dir = meta.is_dir();
            let verdict = self.rules.judge(verdict, &path, is_dir, &mut self.hits);
            if verdict.ships()
                && let Some((kind, size, mode)) = found_kind(&child, &meta, &path)?
            {
                self.items.push(Found {
                    path: path.clone(),
                    local: child.clone(),
                    kind,
                    mode,
                    size,
                });
            }
            if is_dir && self.rules.enter(verdict, &path) {
                self.visit(&child, &path, verdict)?;
            }
        }
        Ok(())
    }
}

/// What a shipped path is: files and links, never folders.
type FoundKind = Option<(Kind, u64, u32)>;

fn found_kind(child: &Path, meta: &std::fs::Metadata, path: &str) -> Result<FoundKind> {
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(child)
            .with_context(|| format!("reading the link {}", child.display()))?;
        #[cfg(windows)]
        let target = link_target(child, target)?;
        return Ok(Some((Kind::Link(target), 0, 0o777)));
    }
    if meta.is_file() {
        let mode = if path.rsplit('/').next().unwrap_or(path).starts_with(".env") {
            // Secrets stay readable by their owner only.
            0o600
        } else {
            file_mode(meta)
        };
        return Ok(Some((Kind::File, meta.len(), mode)));
    }
    Ok(None)
}

/// Walks a path outside the project folder: one file, or a folder's contents.
fn walk_direct(root: &Path) -> Result<Vec<Found>> {
    let full = crate::config::full(root);
    let meta = std::fs::symlink_metadata(root)
        .with_context(|| format!("{} does not exist", full.display()))?;
    let mut items = Vec::new();
    if meta.is_dir() {
        walk_direct_in(root, root, &mut items)?;
    } else {
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some((kind, size, mode)) = found_kind(root, &meta, &name)? {
            items.push(Found {
                path: name,
                local: root.to_path_buf(),
                kind,
                mode,
                size,
            });
        }
    }
    Ok(items)
}

fn walk_direct_in(root: &Path, dir: &Path, items: &mut Vec<Found>) -> Result<()> {
    let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    children.sort();
    for child in children {
        let name = child
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name == ".git" {
            continue;
        }
        let relative = child
            .strip_prefix(root)
            .map_or_else(|_| name.clone(), paths::slash);
        let meta = std::fs::symlink_metadata(&child)
            .with_context(|| format!("reading {}", child.display()))?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            walk_direct_in(root, &child, items)?;
        } else if let Some((kind, size, mode)) = found_kind(&child, &meta, &relative)? {
            items.push(Found {
                path: relative,
                local: child.clone(),
                kind,
                mode,
                size,
            });
        }
    }
    Ok(())
}

/// The mode a file ships with: on Unix, its own.
#[cfg(unix)]
fn file_mode(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

/// The mode a file ships with: Windows has no modes to copy.
#[cfg(windows)]
fn file_mode(_meta: &std::fs::Metadata) -> u32 {
    0o644
}

/// A link's target as the server reads it: each `\` becomes `/`.
#[cfg(windows)]
fn link_target(link: &Path, target: PathBuf) -> Result<PathBuf> {
    use std::path::Component;
    if target.has_root() || matches!(target.components().next(), Some(Component::Prefix(_))) {
        bail!(
            "the link {} points to {}, a path on this PC; make the link relative to its folder",
            link.display(),
            target.display()
        );
    }
    let text = target.to_str().with_context(|| {
        format!(
            "the link {} points to a name that is not UTF-8",
            link.display()
        )
    })?;
    Ok(PathBuf::from(text.replace('\\', "/")))
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
