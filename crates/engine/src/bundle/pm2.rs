//! The PM2 app file each release carries when shipdeck writes it:
//! `<service>.pm2.json`, from `service.start` and `service.pm2.options`.

use anyhow::Result;
use serde_json::{Map, Value, json};

use super::unit::program_path;
use crate::config::{Deployable, is_node, split_args};

/// How PM2 starts the program in `service.start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub script: String,
    pub args: Vec<String>,
    /// `none` for a program PM2 runs as it is.
    pub interpreter: String,
}

/// `node <script> [args]` runs the script with node as PM2's interpreter,
/// so cluster mode works; node's own flags go in `pm2.options.node_args`
/// (the rules see to it). Any other program runs as it is.
pub fn launch(deployable: &Deployable, start_path: Option<&str>) -> Launch {
    let (program, rest) = deployable.start();
    let is_node = is_node(&program);
    let path = program_path(deployable, program, start_path);
    // The rules refuse an unclosed quote before this runs.
    let mut words = split_args(&rest).unwrap_or_default();
    if is_node && !words.is_empty() {
        let script = words.remove(0);
        return Launch {
            script,
            args: words,
            interpreter: path,
        };
    }
    Launch {
        script: path,
        args: words,
        interpreter: "none".into(),
    }
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
    for (key, value) in &service.pm2.options {
        app.insert(key.clone(), value.clone());
    }
    let file = json!({ "apps": [Value::Object(app)] });
    Ok(serde_json::to_string_pretty(&file)? + "\n")
}
