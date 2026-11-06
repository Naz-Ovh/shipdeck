//! The PM2 app file each release carries when shipdeck writes it:
//! `<service>.pm2.json`, from `service.start` and `service.pm2.options`.

use anyhow::Result;
use serde_json::{Map, Value, json};

use super::ProcessSpec;
use super::unit::{program_path_for, workdir};
use crate::config::{Deployable, Service, is_node, split_args};

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
#[must_use]
pub fn launch(deployable: &Deployable, start_path: Option<&str>) -> Launch {
    launch_in(deployable, start_path, "")
}

/// Resolves the command inside an app's release folder.
fn launch_in(deployable: &Deployable, start_path: Option<&str>, release_dir: &str) -> Launch {
    launch_service(
        &deployable.config.service,
        deployable.binary_name().as_deref(),
        start_path,
        release_dir,
    )
}

/// Resolves a PM2 command from process settings.
fn launch_service(
    service: &Service,
    binary: Option<&str>,
    start_path: Option<&str>,
    release_dir: &str,
) -> Launch {
    let (program, rest) = service.command(binary);
    let is_node = is_node(&program);
    let path = program_path_for(service, program, start_path, release_dir);
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
///
/// # Errors
///
/// Fails when the app file cannot be serialized to JSON.
pub fn render(deployable: &Deployable, start_path: Option<&str>) -> Result<String> {
    render_in(deployable, start_path, "")
}

/// Renders a PM2 file for a folder within the shared release.
pub(super) fn render_in(
    deployable: &Deployable,
    start_path: Option<&str>,
    release_dir: &str,
) -> Result<String> {
    let service = &deployable.config.service;
    render_service(
        service,
        deployable.binary_name().as_deref(),
        start_path,
        release_dir,
    )
}

/// Renders PM2 settings from a process specification.
pub(super) fn render_spec(spec: &ProcessSpec<'_>, start_path: Option<&str>) -> Result<String> {
    render_service(spec.service, spec.binary_name, start_path, spec.release_dir)
}

/// Creates the PM2 file for the given service and working folder.
fn render_service(
    service: &Service,
    binary: Option<&str>,
    start_path: Option<&str>,
    release_dir: &str,
) -> Result<String> {
    let launch = launch_service(service, binary, start_path, release_dir);
    let mut app = Map::new();
    app.insert("name".into(), json!(service.pm2_app()));
    app.insert("cwd".into(), json!(workdir(service, release_dir)));
    app.insert("script".into(), json!(launch.script));
    app.insert("args".into(), json!(launch.args));
    app.insert("interpreter".into(), json!(launch.interpreter));
    for (key, value) in &service.pm2.options {
        app.insert(key.clone(), value.clone());
    }
    let file = json!({ "apps": [Value::Object(app)] });
    Ok(serde_json::to_string_pretty(&file)? + "\n")
}
