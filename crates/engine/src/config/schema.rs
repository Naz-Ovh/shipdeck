//! The JSON Schema for `deploy.yaml`, generated from the config types. It
//! is published in `docs/schema/` at each release tag for editor completion,
//! and shipdeck checks every deploy.yaml against it at launch.

use schemars::generate::SchemaSettings;
use serde_json::Value;

use super::DeployConfig;

/// The file in the shipdeck repository, published with each release.
pub const FILE_NAME: &str = "docs/schema/deploy.schema.json";
/// The same file, served by the website at `/deploy.schema.json`.
pub const WEBSITE_COPY: &str = "docs/website/public/deploy.schema.json";

/// Where editors fetch the schema for this version of shipdeck.
#[must_use]
pub fn url() -> String {
    crate::setup::raw_url(FILE_NAME)
}

/// The line that gives a deploy.yaml completion in editors using the YAML
/// language server (VS Code's YAML extension, Neovim, Zed, `JetBrains`).
#[must_use]
pub fn modeline() -> String {
    format!("# yaml-language-server: $schema={}", url())
}

/// The schema as JSON. Draft-07, which every YAML language server reads.
#[must_use]
pub fn generate() -> Value {
    let schema = SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<DeployConfig>();
    let mut value = schema.to_value();
    unwrap_lines(&mut value);
    if let Some(object) = value.as_object_mut() {
        object.insert("$id".into(), Value::String(url()));
        object.insert("title".into(), Value::String("shipdeck deploy.yaml".into()));
    }
    value
}

/// Doc comments arrive wrapped at 80 columns; editors show descriptions as
/// flowing text, so the breaks go.
fn unwrap_lines(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                match child {
                    Value::String(text) if key == "description" => {
                        *text = text.split('\n').collect::<Vec<_>>().join(" ");
                    }
                    _ => unwrap_lines(child),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(unwrap_lines),
        _ => {}
    }
}

/// The schema as the text of `deploy.schema.json`.
///
/// # Panics
/// Panics when the generated schema cannot be serialized as JSON.
#[must_use]
pub fn text() -> String {
    let mut text = serde_json::to_string_pretty(&generate()).expect("a schema is plain JSON");
    text.push('\n');
    text
}

#[cfg(test)]
#[path = "schema_test.rs"]
mod tests;
