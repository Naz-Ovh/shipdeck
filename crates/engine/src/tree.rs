//! `shipdeck tree`: every path the next release will hold, from local files
//! only. It uses the same plan as `deploy`, so the two always agree.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::Result;

use crate::bundle::{self, Entry, Origin, Plan, Source};
use crate::config::Deployment;
use crate::deploy::commands::Exec;
use crate::deploy::report::Reporter;
use crate::deploy::{Action, Step};
use crate::human;
use crate::release::{Manifest, ReleaseName};

/// What `shipdeck tree` was asked for.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Only one app's folder.
    pub app: Option<String>,
    /// Fold folders deeper than this.
    pub depth: Option<usize>,
    /// Run the build steps first.
    pub build: bool,
    /// One JSON object per path.
    pub json: bool,
}

/// The tree, and the include patterns that matched nothing.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub text: String,
    pub missing: Vec<String>,
}

/// Prints the plan for `deployment`.
///
/// # Errors
/// Returns an error when the files cannot be collected, or a build step fails.
pub async fn run(deployment: &Deployment, options: &Options) -> Result<Outcome> {
    if options.build {
        Box::pin(build(deployment)).await?;
    }
    let (name, _) = crate::build::release_name(deployment).await?;
    let settings = deployment.settings()?;
    let manifest = Manifest::new(&name, "shipdeck tree");
    let json = serde_json::to_vec_pretty(&manifest)?;
    let plan = bundle::release_plan(deployment, &name, &json, &settings, &BTreeMap::new())?;
    let missing = plan.missing.clone();
    let plan = match &options.app {
        Some(app) => only_app(&plan, app),
        None => plan,
    };
    let text = if options.json {
        json_tree(&plan, options)
    } else {
        text_tree(deployment, &name, &plan, options)
    };
    Ok(Outcome { text, missing })
}

/// Runs the local build commands, release first, then the apps.
async fn build(deployment: &Deployment) -> Result<()> {
    let (name, _) = crate::build::release_name(deployment).await?;
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let report = Reporter::new(0, tx);
    let exec = Exec {
        deployment,
        report: &report,
        session: None,
        release: Some(name.id()),
        version: Some(name.version.clone()),
        previous: None,
        failed_step: None,
        action: Action::Deploy,
        row: 0,
    };
    for scope in crate::deploy::scopes(deployment, Step::Build) {
        Box::pin(exec.local(Step::Build, scope, None)).await?;
    }
    Ok(())
}

/// The part of a plan that belongs to one app.
fn only_app(plan: &Plan, app: &str) -> Plan {
    let folder = format!("apps/{app}");
    let keep = |path: &str| path == folder || path.starts_with(&format!("{folder}/"));
    let entries: Vec<Entry> = plan
        .entries
        .iter()
        .filter(|entry| keep(&entry.path))
        .map(|entry| {
            let mut entry = entry.clone();
            entry.path = strip(&entry.path, &folder);
            entry
        })
        .collect();
    let links = plan
        .links
        .iter()
        .filter(|link| keep(&link.path))
        .map(|link| bundle::Link {
            path: strip(&link.path, &folder),
            target: link.target.clone(),
        })
        .collect();
    let roots = plan
        .roots
        .iter()
        .filter(|root| keep(root))
        .map(|root| strip(root, &folder))
        .collect();
    Plan {
        entries,
        links,
        missing: plan.missing.clone(),
        roots,
    }
}

/// A path with its leading `folder/` removed.
fn strip(path: &str, folder: &str) -> String {
    path.strip_prefix(folder)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .to_string()
}

/// One node of the printed tree.
#[derive(Debug, Default)]
struct Node {
    kind: Kind,
    origin: Option<Origin>,
    local: Option<PathBuf>,
    mode: u32,
    size: u64,
    target: Option<String>,
    children: BTreeMap<String, Node>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    Dir,
    File,
    Link,
}

/// Builds the tree nodes from a plan.
fn nodes(plan: &Plan) -> Node {
    let mut root = Node::default();
    for entry in &plan.entries {
        let (kind, target, mode, size) = match &entry.source {
            Source::File(_) | Source::Bytes(_) => (Kind::File, None, entry.mode, entry.size),
            Source::Link(target) => (Kind::Link, Some(target.display().to_string()), 0o777, 0),
        };
        let local = match &entry.source {
            Source::File(local) => Some(local.clone()),
            _ => None,
        };
        insert(
            &mut root,
            &entry.path,
            Node {
                kind,
                origin: Some(entry.origin.clone()),
                local,
                mode,
                size,
                target,
                children: BTreeMap::new(),
            },
        );
    }
    for link in &plan.links {
        insert(
            &mut root,
            &link.path,
            Node {
                kind: Kind::Link,
                origin: None,
                local: None,
                mode: 0o777,
                size: 0,
                target: Some(link.target.clone()),
                children: BTreeMap::new(),
            },
        );
    }
    // `.shipdeck/` is the one generated folder the tree names.
    if let Some(shipdeck) = root.children.get_mut(".shipdeck") {
        shipdeck.origin = Some(Origin::Generated);
    }
    root
}

fn insert(root: &mut Node, path: &str, leaf: Node) {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let mut node = root;
    for (index, part) in parts.iter().enumerate() {
        node = node.children.entry((*part).to_string()).or_default();
        let _ = index;
    }
    let (kind, origin, local, mode, size, target) = (
        leaf.kind,
        leaf.origin,
        leaf.local,
        leaf.mode,
        leaf.size,
        leaf.target,
    );
    node.kind = kind;
    node.origin = origin;
    node.local = local;
    node.mode = mode;
    node.size = size;
    node.target = target;
}

/// The printed tree.
fn text_tree(
    deployment: &Deployment,
    name: &ReleaseName,
    plan: &Plan,
    options: &Options,
) -> String {
    let root = nodes(plan);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}/releases/{}   (current after the switch)\n.",
        deployment.dir(),
        name.id()
    );
    render(&mut out, &root, "", "", plan, options);
    let files = plan
        .entries
        .iter()
        .filter(|entry| !matches!(entry.source, Source::Link(_)))
        .count();
    let size: u64 = plan
        .entries
        .iter()
        .filter(|entry| !matches!(entry.source, Source::Link(_)))
        .map(|entry| entry.size)
        .sum();
    let _ = writeln!(
        out,
        "\n{files} files, {}.
",
        human::bytes(size)
    );
    out
}

fn render(out: &mut String, node: &Node, prefix: &str, path: &str, plan: &Plan, options: &Options) {
    let children: Vec<(&String, &Node)> = node.children.iter().collect();
    for (index, (name, child)) in children.iter().enumerate() {
        let last = index + 1 == children.len();
        let branch = if last { "└── " } else { "├── " };
        let path = if path.is_empty() {
            (*name).clone()
        } else {
            format!("{path}/{name}")
        };
        let depth = path.split('/').count();
        let fold = child.kind == Kind::Dir
            && options
                .depth
                .is_some_and(|limit| plan.roots.contains(&path) || depth == limit)
            && !child.children.is_empty();
        let (left, label) = if fold {
            (
                format!("{name}/ ({} files)", files_below(child)),
                label_of_dir(child, &path, plan),
            )
        } else {
            match child.kind {
                Kind::Dir => (format!("{name}/"), label_of(child, &path)),
                Kind::File => ((*name).clone(), label_of(child, &path)),
                Kind::Link => (
                    format!("{name} -> {}", child.target.as_deref().unwrap_or("")),
                    "link".to_string(),
                ),
            }
        };
        let _ = writeln!(out, "{prefix}{branch}{}", aligned(&left, &label));
        if child.kind == Kind::Dir && !fold {
            let next = format!("{prefix}{}", if last { "    " } else { "│   " });
            render(out, child, &next, &path, plan, options);
        }
    }
}

/// A name and its right-hand column, lined up.
fn aligned(left: &str, label: &str) -> String {
    if label.is_empty() {
        left.to_string()
    } else if left.chars().count() >= 50 {
        format!("{left}  {label}")
    } else {
        format!("{left:<50}  {label}")
    }
}

fn label_of(node: &Node, path: &str) -> String {
    if path == ".shipdeck" {
        return "generated".to_string();
    }
    if path.starts_with(".shipdeck/") {
        return String::new();
    }
    node.origin.as_ref().map(Origin::label).unwrap_or_default()
}

fn label_of_dir(node: &Node, path: &str, plan: &Plan) -> String {
    if path == ".shipdeck" {
        return "generated".to_string();
    }
    if let Some(origin) = &node.origin
        && !matches!(origin, Origin::Generated)
    {
        return origin.label();
    }
    let prefix = format!("{path}/");
    plan.entries
        .iter()
        .find(|entry| entry.path.starts_with(&prefix) && !matches!(entry.origin, Origin::Generated))
        .map(|entry| entry.origin.label())
        .unwrap_or_default()
}

fn files_below(node: &Node) -> usize {
    node.children
        .values()
        .map(|child| match child.kind {
            Kind::File => 1,
            Kind::Link => 0,
            Kind::Dir => files_below(child),
        })
        .sum()
}

fn size_below(node: &Node) -> u64 {
    node.children
        .values()
        .map(|child| match child.kind {
            Kind::Dir => size_below(child),
            Kind::File => child.size,
            Kind::Link => 0,
        })
        .sum()
}

/// One JSON object per printed path.
fn json_tree(plan: &Plan, options: &Options) -> String {
    let root = nodes(plan);
    let mut out = String::new();
    json_render(&mut out, &root, "", plan, options);
    out
}

fn json_render(out: &mut String, node: &Node, prefix: &str, plan: &Plan, options: &Options) {
    for (name, child) in &node.children {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let depth = path.split('/').count();
        let fold = child.kind == Kind::Dir
            && options
                .depth
                .is_some_and(|limit| plan.roots.contains(&path) || depth == limit)
            && !child.children.is_empty();
        let object = match child.kind {
            Kind::Dir if fold => serde_json::json!({
                "path": path,
                "kind": "dir",
                "from": json_origin(&label_of_dir(child, &path, plan)),
                "local": serde_json::Value::Null,
                "mode": "0755",
                "size": size_below(child),
                "files": files_below(child),
            }),
            Kind::Dir => serde_json::json!({
                "path": path,
                "kind": "dir",
                "from": json_origin(&label_of(child, &path)),
                "local": serde_json::Value::Null,
                "mode": "0755",
                "size": size_below(child),
            }),
            Kind::Link => serde_json::json!({
                "path": path,
                "kind": "link",
                "from": "link",
                "local": serde_json::Value::Null,
                "mode": format!("{:04o}", child.mode),
                "size": 0,
                "target": child.target,
            }),
            Kind::File => serde_json::json!({
                "path": path,
                "kind": "file",
                "from": json_origin(&label_of(child, &path)),
                "local": child.local.as_ref().map(|path| path.display().to_string()),
                "mode": format!("{:04o}", child.mode),
                "size": child.size,
            }),
        };
        out.push_str(&serde_json::to_string(&object).unwrap_or_default());
        out.push('\n');
        if child.kind == Kind::Dir && !fold {
            json_render(out, child, &path, plan, options);
        }
    }
}

fn json_origin(label: &str) -> serde_json::Value {
    if label.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(label.to_string())
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
