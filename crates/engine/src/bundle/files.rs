//! Which files go into a release: `pipeline.pack.files` applied to the
//! project and to the deploy folder.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::glob::{Rules, Verdict};
use crate::config::{Deployable, FILE_NAMES, UNIT_TEMPLATE, release_path};

/// Never shipped, at any depth.
const ALWAYS_LEFT_OUT: &str = ".git";

/// Where files come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Project,
    Deploy,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Deploy => "deploy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    File {
        size: u64,
        mode: u32,
    },
    Dir,
    /// Kept as a link, so `node_modules/.bin` and friends stay intact.
    Link(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub local: PathBuf,
    pub kind: Kind,
    pub source: Source,
}

/// What `pipeline.pack.files` picks, by path in the release.
#[derive(Debug, Default)]
pub struct Selection {
    pub items: BTreeMap<String, Item>,
    /// Project files a deploy-folder file replaced.
    pub replaced: Vec<String>,
}

impl Selection {
    pub fn files(&self) -> usize {
        self.items
            .values()
            .filter(|i| matches!(i.kind, Kind::File { .. }))
            .count()
    }
}

/// The folder a source is read from, and its patterns.
fn source(deployable: &Deployable, source: Source) -> Result<(PathBuf, Rules)> {
    let files = &deployable.config.pipeline.pack.files;
    let (dir, include, exclude) = match source {
        Source::Project => (
            deployable.project_dir(),
            &files.include.project,
            &files.exclude.project,
        ),
        Source::Deploy => (
            deployable.folder.clone(),
            &files.include.deploy,
            &files.exclude.deploy,
        ),
    };
    let rules = Rules::parse(include, exclude)
        .map_err(|why| anyhow::anyhow!("pipeline.pack.files.*.{}: {why}", source.name()))?;
    Ok((dir, rules))
}

/// Picks every file `pipeline.pack.files` names. Fails when a pattern finds
/// nothing, so a missing build output is caught before anything ships.
pub fn select(deployable: &Deployable) -> Result<Selection> {
    let mut selection = Selection::default();
    for from in [Source::Project, Source::Deploy] {
        let walked = walk_source(deployable, from)?;
        for (path, item) in walked {
            match selection.items.get(&path) {
                None => {}
                Some(old) if old.kind == Kind::Dir && item.kind == Kind::Dir => continue,
                Some(old)
                    if matches!(old.kind, Kind::File { .. } | Kind::Link(_))
                        && matches!(item.kind, Kind::File { .. } | Kind::Link(_)) =>
                {
                    selection.replaced.push(path.clone());
                }
                Some(_) => bail!(
                    "{path} is a folder in one source and a file in the other; \
                     the deploy folder can replace a project file, not a folder"
                ),
            }
            selection.items.insert(path, item);
        }
    }
    Ok(selection)
}

/// Walks one source and checks every pattern found something.
pub fn walk_source(deployable: &Deployable, from: Source) -> Result<BTreeMap<String, Item>> {
    let (dir, rules) = source(deployable, from)?;
    if rules.include.is_empty() {
        return Ok(BTreeMap::new());
    }
    if !dir.is_dir() {
        bail!(
            "the {} folder {} does not exist",
            from.name(),
            dir.display()
        );
    }
    let skip: &[&str] = match from {
        Source::Project => &[],
        Source::Deploy => &[FILE_NAMES[0], FILE_NAMES[1], UNIT_TEMPLATE],
    };
    let mut walk = Walk {
        rules: &rules,
        skip,
        source: from,
        hits: vec![0; rules.include.len()],
        items: BTreeMap::new(),
    };
    let verdict = rules.judge(Verdict::default(), "", true, &mut walk.hits);
    walk.visit(&dir, "", verdict)?;

    let missing: Vec<String> = rules
        .include
        .iter()
        .zip(&walk.hits)
        .filter(|(pattern, hits)| !pattern.negated && **hits == 0)
        .map(|(pattern, _)| match pattern.literal() {
            Some(path) => format!("{} (not found)", release_path(&path)),
            None => format!("{} (matched nothing)", pattern.text),
        })
        .collect();
    if !missing.is_empty() {
        bail!(
            "pipeline.pack.files.include.{} in {}: {}",
            from.name(),
            dir.display(),
            missing.join(", ")
        );
    }
    Ok(walk.items)
}

struct Walk<'a> {
    rules: &'a Rules,
    skip: &'a [&'a str],
    source: Source,
    hits: Vec<usize>,
    items: BTreeMap<String, Item>,
}

impl Walk<'_> {
    fn visit(&mut self, dir: &Path, rel: &str, verdict: Verdict) -> Result<()> {
        let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        children.sort();
        for child in children {
            let name = child
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name == ALWAYS_LEFT_OUT || (rel.is_empty() && self.skip.contains(&name.as_str())) {
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
                    .any(|p| p.literal().is_some_and(|l| release_path(&l) == path))
            {
                meta = std::fs::metadata(&child)
                    .with_context(|| format!("following the link {}", child.display()))?;
            }
            let is_dir = meta.is_dir();
            let verdict = self.rules.judge(verdict, &path, is_dir, &mut self.hits);
            if verdict.ships() {
                let kind = if meta.file_type().is_symlink() {
                    Some(Kind::Link(std::fs::read_link(&child).with_context(
                        || format!("reading the link {}", child.display()),
                    )?))
                } else if is_dir {
                    Some(Kind::Dir)
                } else if meta.is_file() {
                    Some(Kind::File {
                        size: meta.len(),
                        mode: if path.rsplit('/').next().unwrap_or(&path).starts_with(".env") {
                            // Secrets stay readable by their owner only.
                            0o600
                        } else {
                            meta.permissions().mode() & 0o777
                        },
                    })
                } else {
                    // Sockets and pipes mean nothing on another machine.
                    None
                };
                if let Some(kind) = kind {
                    self.items.insert(
                        path.clone(),
                        Item {
                            local: child.clone(),
                            kind,
                            source: self.source,
                        },
                    );
                }
            }
            if is_dir && self.rules.enter(verdict, &path) {
                self.visit(&child, &path, verdict)?;
            }
        }
        Ok(())
    }
}
