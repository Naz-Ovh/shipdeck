//! The JSON Schema for the root file and the deploy file, generated from
//! their types. It is published in `docs/schema/` at each release tag, for
//! editor completion, and every file is checked against it when read.

use schemars::generate::SchemaSettings;
use serde_json::Value;

use super::types::{DeployFile, RootFile};

/// The file in the shipdeck repository, published with each release.
pub const FILE_NAME: &str = "docs/schema/deploy.schema.json";
/// The same file, served by the website at `/deploy.schema.json`.
pub const WEBSITE_COPY: &str = "docs/website/public/deploy.schema.json";

/// Where editors fetch the schema for this version of shipdeck.
#[must_use]
pub fn url() -> String {
    crate::setup::raw_url(FILE_NAME)
}

/// The line that gives a deploy file completion in editors using the YAML
/// language server (VS Code's YAML extension, Neovim, Zed, `JetBrains`).
#[must_use]
pub fn modeline() -> String {
    format!("# yaml-language-server: $schema={}", url())
}

/// The schema of a deploy file.
#[must_use]
pub fn deploy() -> Value {
    root::<DeployFile>("deploy file")
}

/// The schema of the root file.
#[must_use]
pub fn root_file() -> Value {
    root::<RootFile>("root file")
}

/// The schema of either file, which is what the published file holds.
#[must_use]
pub fn published() -> Value {
    let mut deploy = deploy();
    let mut project = root_file();
    let mut all = definitions(&mut deploy);
    all.extend(definitions(&mut project));
    for schema in [&mut deploy, &mut project] {
        if let Some(object) = schema.as_object_mut() {
            object.remove("definitions");
        }
    }
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": url(),
        "title": "shipdeck",
        "description": "The root file (shipdeck.yaml or .shipdeck) and the deploy file.",
        "oneOf": [project, deploy],
        "definitions": all,
    })
}

fn root<T: schemars::JsonSchema>(title: &str) -> Value {
    let schema = SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>();
    let mut value = schema.to_value();
    unwrap_lines(&mut value);
    if let Some(object) = value.as_object_mut() {
        object.insert("$id".into(), Value::String(url()));
        object.insert("title".into(), Value::String(title.into()));
    }
    value
}

fn definitions(schema: &mut Value) -> serde_json::Map<String, Value> {
    schema
        .as_object_mut()
        .and_then(|object| object.remove("definitions"))
        .and_then(|definitions| definitions.as_object().cloned())
        .unwrap_or_default()
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
    let mut text = serde_json::to_string_pretty(&published()).expect("a schema is plain JSON");
    text.push('\n');
    text
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
