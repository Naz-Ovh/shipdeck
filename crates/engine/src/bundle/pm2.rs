//! The PM2 app file each release carries in `.shipdeck/`, from
//! `process.start` and `process.pm2.options`.

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use crate::config::{Deployment, Program, file_name};
use crate::release::ReleaseName;

/// The app's PM2 file in the release.
#[must_use]
pub fn pm2_file(deployment: &Deployment, app: &str) -> String {
    format!(".shipdeck/{}.pm2.json", deployment.process_name(app))
}

/// How PM2 starts the app.
struct Launch {
    script: String,
    args: Vec<String>,
    /// `none` for a program PM2 runs as it is.
    interpreter: String,
}

/// `node <script> [args]` runs the script with node as PM2's interpreter,
/// so cluster mode works; any other program runs as it is.
fn launch(deployment: &Deployment, app: &str) -> Result<Launch> {
    let (program, words) = deployment
        .start_words(app)
        .with_context(|| format!("app {app} has an unclosed quote in process.start"))?;
    let current = format!("{}/current/apps/{app}", deployment.dir());
    let path = match program {
        Program::Absolute(path) => path,
        Program::InRelease(path) => format!("{current}/{path}"),
        Program::Tool(name) => name,
    };
    if file_name(&path) == "node"
        && let Some((script, args)) = words.split_first()
    {
        return Ok(Launch {
            script: script.clone(),
            args: args.to_vec(),
            interpreter: path,
        });
    }
    Ok(Launch {
        script: path,
        args: words,
        interpreter: "none".into(),
    })
}

/// Renders the app's PM2 file. The environment is not in it: shipdeck
/// exports `process.env` and `process.env_file` before it calls PM2.
///
/// # Errors
/// Fails when the app cannot start or the file cannot be serialized.
pub fn render(deployment: &Deployment, app: &str, release: &ReleaseName) -> Result<String> {
    let launch = launch(deployment, app)?;
    let id = release.id();
    let fill = |text: &str| deployment.expand(text, Some(&id), Some(&release.version), Some(app));
    let mut process = Map::new();
    process.insert("name".into(), json!(deployment.process_name(app)));
    process.insert(
        "cwd".into(),
        json!(format!("{}/current/apps/{app}", deployment.dir())),
    );
    process.insert("script".into(), json!(fill(&launch.script)?));
    process.insert("args".into(), json!(launch.args));
    process.insert("interpreter".into(), json!(fill(&launch.interpreter)?));
    if let Some(pm2) = deployment.pm2(app) {
        for (key, setting) in &pm2.options {
            process.insert(key.clone(), fill_options(setting, &fill)?);
        }
    }
    let json_file = json!({ "apps": [Value::Object(process)] });
    Ok(serde_json::to_string_pretty(&json_file)? + "\n")
}

/// A PM2 option with every string inside it expanded.
fn fill_options(value: &Value, fill: &impl Fn(&str) -> Result<String>) -> Result<Value> {
    Ok(match value {
        Value::String(text) => Value::String(fill(text)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| fill_options(item, fill))
                .collect::<Result<Vec<_>>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| Ok((key.clone(), fill_options(item, fill)?)))
                .collect::<Result<Map<_, _>>>()?,
        ),
        other => other.clone(),
    })
}
