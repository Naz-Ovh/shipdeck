//! `shipdeck man`: the manual, for people and coding agents alike.
//! `man index` lists the topics, `man configs` every deploy.yaml property,
//! and `man details <KEY>...` prints topics and properties in full, each in
//! its own tagged block. The text is `manual/*.yaml`; `build.rs` turns it
//! into the static data below, so a run reads no file and parses nothing.

use std::fmt::Write as _;
use std::process::ExitCode;

use clap::{Args, Subcommand};

/// A group of topics in the index.
pub struct Section {
    pub name: &'static str,
    pub topics: &'static [Topic],
}

/// One page of the manual.
pub struct Topic {
    /// What `man details` takes, such as `step.health`.
    pub key: &'static str,
    /// A few words, for the index.
    pub brief: &'static str,
    /// The keys of related topics.
    pub see: &'static [&'static str],
    pub text: &'static str,
}

/// One deploy.yaml property, in the order of the tree.
pub struct Config {
    /// Its dotted path, such as `service.env`.
    pub key: &'static str,
    /// How deep it sits: 0 for a top-level key.
    pub depth: usize,
    /// Its value and default, such as `bool, default true`; empty for a block.
    pub kind: &'static str,
    pub brief: &'static str,
    pub see: &'static [&'static str],
    /// More than the brief says; may be empty.
    pub text: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/manual.rs"));

/// Usage errors, as the flag parser reports them.
const USAGE: u8 = 2;

/// The last line of `man details`: where the documentation goes on.
const MORE: &str = "For more, check out https://forgedoc.naz.ovh/llms.txt";

#[derive(Args)]
pub struct ManArgs {
    #[command(subcommand)]
    page: Option<Page>,
}

#[derive(Subcommand)]
enum Page {
    /// Every topic as `key: brief`; `man` alone prints it too
    Index,
    /// Every deploy.yaml property as a YAML tree, with its type, default and brief
    Configs,
    /// Topics and properties in full, each in its own tagged block
    Details {
        /// Topic keys or property paths, apart or comma-separated: `step.health service.env`
        /// or `step.health,service.env`
        #[arg(value_name = "KEY", required = true)]
        keys: Vec<String>,
    },
}

pub fn run(args: ManArgs) -> ExitCode {
    let text = match args.page {
        None | Some(Page::Index) => index(),
        Some(Page::Configs) => configs(),
        Some(Page::Details { keys }) => match details(&keys) {
            Ok(text) => text,
            Err(errors) => {
                eprint!("{errors}");
                return ExitCode::from(USAGE);
            }
        },
    };
    print!("{text}");
    ExitCode::SUCCESS
}

fn topics() -> impl Iterator<Item = &'static Topic> {
    SECTIONS.iter().flat_map(|section| section.topics)
}

/// Every key `man details` takes.
fn keys() -> impl Iterator<Item = &'static str> {
    topics()
        .map(|topic| topic.key)
        .chain(CONFIGS.iter().map(|config| config.key))
}

fn index() -> String {
    let mut out = format!(
        "shipdeck {} manual. Topics as `key: brief`.\n\
         Read in full: shipdeck man details <key> [<key>...]  (several: spaces or commas)\n\
         deploy.yaml properties: shipdeck man configs; one in full: shipdeck man details <path>\n",
        env!("CARGO_PKG_VERSION"),
    );
    for section in SECTIONS {
        let _ = writeln!(out, "\n[{}]", section.name);
        for topic in section.topics {
            let _ = writeln!(out, "{}: {}", topic.key, topic.brief);
        }
    }
    out
}

fn configs() -> String {
    let mut out = format!(
        "# deploy.yaml properties (shipdeck {}). Each line: `name: type, default  # brief`.\n\
         # commands = one shell line or a list run in order; patterns = paths or globs\n\
         # (man details config.files). One in full: shipdeck man details <path>, e.g. service.env\n",
        env!("CARGO_PKG_VERSION"),
    );
    out.push_str(&tree(CONFIGS, 0));
    out
}

/// `configs` as YAML lines, indented from `depth` down.
fn tree(configs: &[Config], depth: usize) -> String {
    let mut out = String::new();
    for config in configs {
        let name = config.key.rsplit('.').next().unwrap_or(config.key);
        let indent = "  ".repeat(config.depth - depth);
        let value = if config.kind.is_empty() {
            String::new()
        } else {
            format!(" {}", config.kind)
        };
        let _ = writeln!(out, "{indent}{name}:{value}  # {}", config.brief);
    }
    out
}

/// The blocks for `args`, each split at commas and spaces, in order; or
/// every key that is neither a topic nor a property.
fn details(args: &[String]) -> Result<String, String> {
    let wanted: Vec<&str> = args
        .iter()
        .flat_map(|arg| arg.split(|c: char| c == ',' || c.is_whitespace()))
        .filter(|key| !key.is_empty())
        .collect();
    let mut blocks = Vec::new();
    let mut errors = String::new();
    for key in &wanted {
        if let Some(topic) = topics().find(|topic| topic.key == *key) {
            blocks.push(topic_block(topic));
        } else if let Some(at) = CONFIGS.iter().position(|config| config.key == *key) {
            blocks.push(config_block(at));
        } else {
            let close = suggest(key);
            let hint = if close.is_empty() {
                String::new()
            } else {
                format!("; did you mean {}?", close.join(", "))
            };
            let _ = writeln!(errors, "error: `{key}` is no topic or property{hint}");
        }
    }
    if errors.is_empty() {
        Ok(format!("{}\n{MORE}\n", blocks.join("\n")))
    } else {
        errors.push_str(
            "Topics: shipdeck man index. Properties: shipdeck man configs. Nothing was printed.\n",
        );
        Err(errors)
    }
}

fn topic_block(topic: &Topic) -> String {
    let mut out = format!(
        "<topic key=\"{}\" brief=\"{}\">\n{}\n",
        topic.key, topic.brief, topic.text
    );
    see(&mut out, topic.see);
    out.push_str("</topic>\n");
    out
}

/// The property at `at` in [`CONFIGS`]; a block lists what is in it.
fn config_block(at: usize) -> String {
    let config = &CONFIGS[at];
    let mut out = format!("<config key=\"{}\"", config.key);
    if !config.kind.is_empty() {
        let _ = write!(out, " type=\"{}\"", config.kind);
    }
    let _ = writeln!(out, " brief=\"{}\">", config.brief);
    if !config.text.is_empty() {
        let _ = writeln!(out, "{}", config.text);
    }
    let inside = CONFIGS[at + 1..]
        .iter()
        .position(|c| c.depth <= config.depth)
        .map_or(CONFIGS.len(), |end| at + 1 + end);
    if inside > at + 1 {
        let _ = write!(
            out,
            "fields:\n{}",
            tree(&CONFIGS[at + 1..inside], config.depth)
        );
    }
    see(&mut out, config.see);
    out.push_str("</config>\n");
    out
}

fn see(out: &mut String, keys: &[&str]) {
    if !keys.is_empty() {
        let _ = writeln!(out, "see: {}", keys.join(", "));
    }
}

/// Up to five keys close to `word`: one that holds it as a part, such as
/// `service.env` for `env`, or one a typo or two away.
fn suggest(word: &str) -> Vec<&'static str> {
    let word = word.to_lowercase();
    keys()
        .filter(|key| key.split(['.', '-']).any(|part| part == word) || distance(key, &word) <= 2)
        .take(5)
        .collect()
}

/// Edits (insert, delete, replace) between `a` and `b`.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(above + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
#[path = "man_test.rs"]
mod tests;
