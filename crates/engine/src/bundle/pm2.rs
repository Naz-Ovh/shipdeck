//! The PM2 app file each release carries when shipdeck writes it:
//! `<service>.pm2.json`, from `service.start` and `service.pm2.options`.

use anyhow::Result;
use serde_json::{Map, Value, json};

use super::unit::program_path;
use crate::config::{Deployable, split_args};

/// Node flags whose value is the next word, so it is not the script.
const NODE_FLAGS_WITH_VALUE: [&str; 8] = [
    "-r",
    "--require",
    "--import",
    "--loader",
    "--experimental-loader",
    "-C",
    "--conditions",
    "--env-file",
];

/// How PM2 starts the program in `service.start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub script: String,
    pub args: Vec<String>,
    /// `none` for a program PM2 runs as it is.
    pub interpreter: String,
    pub node_args: Vec<String>,
}

/// `node dist/main.js` runs `dist/main.js` with node as the interpreter, so
/// PM2's cluster mode works; anything else runs as it is.
pub fn launch(deployable: &Deployable, start_path: Option<&str>) -> Launch {
    let (program, rest) = deployable.start();
    let path = program_path(deployable, program, start_path);
    // The rules refuse an unclosed quote before this runs.
    let words = split_args(&rest).unwrap_or_default();
    let is_node = path == "node" || path.ends_with("/node");
    if is_node && let Some(at) = script_at(&words) {
        return Launch {
            script: words[at].clone(),
            args: words[at + 1..].to_vec(),
            interpreter: path,
            node_args: words[..at].to_vec(),
        };
    }
    Launch {
        script: path,
        args: words,
        interpreter: "none".into(),
        node_args: Vec::new(),
    }
}

/// The first word of a node command line that is not a flag or a flag's value.
fn script_at(words: &[String]) -> Option<usize> {
    let mut skip = false;
    for (index, word) in words.iter().enumerate() {
        if skip {
            skip = false;
        } else if word.starts_with('-') {
            skip = NODE_FLAGS_WITH_VALUE.contains(&word.as_str());
        } else {
            return Some(index);
        }
    }
    None
}

/// The app file. The environment is not in it: shipdeck exports
/// `service.env` and `service.env_file` before it calls PM2, which reads
/// them with `--update-env`.
pub fn render(deployable: &Deployable, start_path: Option<&str>) -> Result<String> {
    let service = &deployable.config.service;
    let launch = launch(deployable, start_path);
    let mut app = Map::new();
    app.insert("name".into(), json!(service.pm2_app()));
    app.insert("cwd".into(), json!(format!("{}/current", service.dir)));
    app.insert("script".into(), json!(launch.script));
    app.insert("args".into(), json!(launch.args));
    app.insert("interpreter".into(), json!(launch.interpreter));
    if !launch.node_args.is_empty() {
        app.insert("node_args".into(), json!(launch.node_args));
    }
    for (key, value) in &service.pm2.options {
        app.insert(key.clone(), value.clone());
    }
    let file = json!({ "apps": [Value::Object(app)] });
    Ok(serde_json::to_string_pretty(&file)? + "\n")
}
