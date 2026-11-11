//! The JSON Schema of every message, generated from the Rust types. The
//! desktop app's C# types are made from it, so the two sides cannot drift.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value, json};

use crate::message::{
    AgentBackupCreate, AgentBackupCreated, AgentBackupRestore, AgentCleaned, AgentCleanupWork,
    AgentDiscard, AgentDropRequest, AgentEvent, AgentEventAppend, AgentEventSaved, AgentEvents,
    AgentEventsAfter, AgentEventsRecent, AgentFileInfo, AgentFileKind, AgentHello,
    AgentHelloResult, AgentLinkShared, AgentLogLine, AgentLogsEnded, AgentLogsFollow,
    AgentLogsStop, AgentLogsView, AgentManager, AgentOk, AgentPing, AgentRelease,
    AgentReleaseContents, AgentReleaseContentsRequest, AgentReleaseFile, AgentReleaseFiles,
    AgentReleaseFilesRequest, AgentReleaseKind, AgentReleaseRead, AgentReleaseReadRequest,
    AgentReleases, AgentReleasesRequest, AgentService, AgentServiceAction, AgentServiceControl,
    AgentShutdown, AgentStatus, AskRollback, AskSecret, AskTrustHost, AskUnchanged,
    ConnectionClose, DeployStart, DeployableInfo, Done, EventsRead, EventsReadResult, FileInfo,
    HashCurrent, HashCurrentResult, Initialize, InitializeResult, InspectWork, LogsEnded,
    LogsFollow, LogsFollowResult, LogsLine, LogsStop, Manifest, OpFinished, OpLog, OpProgress,
    OpRelease, OpStep, OperationStarted, Problem, ProjectLock, ReleaseContents,
    ReleaseContentsResult, ReleaseInfo, ReleaseName, ReleaseReadFile, ReleaseReadFileResult,
    ReleasesList, ReleasesListResult, RollbackStart, SecretAnswer, ServerFacts, ServerFactsRequest,
    ServerInfoGet, ServiceControl, StatusGet, StatusGetResult, SwitchRelease, UnpackRelease,
    UnpackResult, UploadPrepare, UploadReservation, WorkCleanup, WorkCleanupResult, WorkGet,
    WorkInspection, WorkReport, WorkspaceOpen, WorkspaceOpenResult,
};

/// The file in the shipdeck repository, published with each release.
pub const FILE_NAME: &str = "docs/schema/protocol.schema.json";

/// Where the schema is published for this version.
#[must_use]
pub fn url() -> String {
    let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let path = repository
        .strip_prefix("https://github.com/")
        .unwrap_or(repository);
    format!(
        "https://raw.githubusercontent.com/{path}/v{}/{FILE_NAME}",
        env!("CARGO_PKG_VERSION")
    )
}

/// The schema as JSON. Draft-07, which every language server and code
/// generator reads.
#[must_use]
pub fn generate() -> Value {
    let mut definitions = Map::new();
    for (name, value) in messages() {
        let mut value = value;
        if let Some(object) = value.as_object_mut() {
            object.remove("$schema");
            if let Some(Value::Object(nested)) = object.remove("definitions") {
                for (key, child) in nested {
                    definitions.entry(key).or_insert(child);
                }
            }
        }
        definitions.insert(name.to_string(), value);
    }
    let mut root = Map::new();
    root.insert(
        "$schema".into(),
        json!("http://json-schema.org/draft-07/schema#"),
    );
    root.insert("$id".into(), json!(url()));
    root.insert("title".into(), json!("shipdeck protocol 1"));
    root.insert("definitions".into(), Value::Object(definitions));
    let mut value = Value::Object(root);
    unwrap_lines(&mut value);
    value
}

/// Each message and its param or result type, by the name the schema uses.
fn messages() -> Vec<(&'static str, Value)> {
    let mut messages = agent_messages();
    messages.extend(app_messages());
    messages
}

/// The agent's words, sent over its own SSH channel.
fn agent_messages() -> Vec<(&'static str, Value)> {
    vec![
        ("AgentHello", schema_for::<AgentHello>()),
        ("AgentOk", schema_for::<AgentOk>()),
        ("ProjectLock", schema_for::<ProjectLock>()),
        ("UnpackRelease", schema_for::<UnpackRelease>()),
        ("UnpackResult", schema_for::<UnpackResult>()),
        ("ServerFacts", schema_for::<ServerFacts>()),
        ("ServerFactsRequest", schema_for::<ServerFactsRequest>()),
        ("ServerInfoGet", schema_for::<ServerInfoGet>()),
        ("WorkGet", schema_for::<WorkGet>()),
        ("WorkReport", schema_for::<WorkReport>()),
        ("WorkCleanup", schema_for::<WorkCleanup>()),
        ("WorkCleanupResult", schema_for::<WorkCleanupResult>()),
        ("EventsRead", schema_for::<EventsRead>()),
        ("EventsReadResult", schema_for::<EventsReadResult>()),
        ("AgentService", schema_for::<AgentService>()),
        ("AgentManager", schema_for::<AgentManager>()),
        ("AgentStatus", schema_for::<AgentStatus>()),
        ("AgentServiceAction", schema_for::<AgentServiceAction>()),
        ("AgentServiceControl", schema_for::<AgentServiceControl>()),
        ("AgentReleases", schema_for::<AgentReleases>()),
        ("AgentReleasesRequest", schema_for::<AgentReleasesRequest>()),
        ("AgentRelease", schema_for::<AgentRelease>()),
        ("AgentReleaseKind", schema_for::<AgentReleaseKind>()),
        ("AgentLinkShared", schema_for::<AgentLinkShared>()),
        ("AgentReleaseFile", schema_for::<AgentReleaseFile>()),
        (
            "AgentReleaseFilesRequest",
            schema_for::<AgentReleaseFilesRequest>(),
        ),
        ("AgentReleaseFiles", schema_for::<AgentReleaseFiles>()),
        ("AgentBackupCreate", schema_for::<AgentBackupCreate>()),
        ("AgentBackupCreated", schema_for::<AgentBackupCreated>()),
        ("AgentBackupRestore", schema_for::<AgentBackupRestore>()),
        ("AgentLogsFollow", schema_for::<AgentLogsFollow>()),
        ("AgentLogsView", schema_for::<AgentLogsView>()),
        ("AgentLogsStop", schema_for::<AgentLogsStop>()),
        ("AgentLogLine", schema_for::<AgentLogLine>()),
        ("AgentLogsEnded", schema_for::<AgentLogsEnded>()),
        ("AgentCleanupWork", schema_for::<AgentCleanupWork>()),
        ("AgentCleaned", schema_for::<AgentCleaned>()),
        ("AgentDiscard", schema_for::<AgentDiscard>()),
        ("AgentEventsRecent", schema_for::<AgentEventsRecent>()),
        ("AgentFileInfo", schema_for::<AgentFileInfo>()),
        ("AgentFileKind", schema_for::<AgentFileKind>()),
        (
            "AgentReleaseContentsRequest",
            schema_for::<AgentReleaseContentsRequest>(),
        ),
        ("AgentReleaseContents", schema_for::<AgentReleaseContents>()),
        (
            "AgentReleaseReadRequest",
            schema_for::<AgentReleaseReadRequest>(),
        ),
        ("AgentReleaseRead", schema_for::<AgentReleaseRead>()),
        ("AgentDropRequest", schema_for::<AgentDropRequest>()),
        ("SwitchRelease", schema_for::<SwitchRelease>()),
        ("AgentEvent", schema_for::<AgentEvent>()),
        ("AgentEventAppend", schema_for::<AgentEventAppend>()),
        ("AgentEventSaved", schema_for::<AgentEventSaved>()),
        ("AgentEvents", schema_for::<AgentEvents>()),
        ("AgentEventsAfter", schema_for::<AgentEventsAfter>()),
        ("AgentHelloResult", schema_for::<AgentHelloResult>()),
        ("AgentPing", schema_for::<AgentPing>()),
        ("AgentShutdown", schema_for::<AgentShutdown>()),
        ("HashCurrent", schema_for::<HashCurrent>()),
        ("HashCurrentResult", schema_for::<HashCurrentResult>()),
        ("InspectWork", schema_for::<InspectWork>()),
        ("WorkInspection", schema_for::<WorkInspection>()),
        ("UploadPrepare", schema_for::<UploadPrepare>()),
        ("UploadReservation", schema_for::<UploadReservation>()),
    ]
}

/// The app protocol's words, sent between an app and the engine.
fn app_messages() -> Vec<(&'static str, Value)> {
    vec![
        ("Initialize", schema_for::<Initialize>()),
        ("InitializeResult", schema_for::<InitializeResult>()),
        ("Done", schema_for::<Done>()),
        ("WorkspaceOpen", schema_for::<WorkspaceOpen>()),
        ("WorkspaceOpenResult", schema_for::<WorkspaceOpenResult>()),
        ("DeployableInfo", schema_for::<DeployableInfo>()),
        ("Problem", schema_for::<Problem>()),
        ("StatusGet", schema_for::<StatusGet>()),
        ("StatusGetResult", schema_for::<StatusGetResult>()),
        ("ConnectionClose", schema_for::<ConnectionClose>()),
        ("ServiceControl", schema_for::<ServiceControl>()),
        ("ReleasesList", schema_for::<ReleasesList>()),
        ("ReleasesListResult", schema_for::<ReleasesListResult>()),
        ("ReleaseInfo", schema_for::<ReleaseInfo>()),
        ("ReleaseName", schema_for::<ReleaseName>()),
        ("Manifest", schema_for::<Manifest>()),
        ("ReleaseContents", schema_for::<ReleaseContents>()),
        (
            "ReleaseContentsResult",
            schema_for::<ReleaseContentsResult>(),
        ),
        ("FileInfo", schema_for::<FileInfo>()),
        ("ReleaseReadFile", schema_for::<ReleaseReadFile>()),
        (
            "ReleaseReadFileResult",
            schema_for::<ReleaseReadFileResult>(),
        ),
        ("LogsFollow", schema_for::<LogsFollow>()),
        ("LogsFollowResult", schema_for::<LogsFollowResult>()),
        ("LogsStop", schema_for::<LogsStop>()),
        ("DeployStart", schema_for::<DeployStart>()),
        ("RollbackStart", schema_for::<RollbackStart>()),
        ("OperationStarted", schema_for::<OperationStarted>()),
        ("LogsLine", schema_for::<LogsLine>()),
        ("LogsEnded", schema_for::<LogsEnded>()),
        ("OpStep", schema_for::<OpStep>()),
        ("OpProgress", schema_for::<OpProgress>()),
        ("OpLog", schema_for::<OpLog>()),
        ("OpRelease", schema_for::<OpRelease>()),
        ("OpFinished", schema_for::<OpFinished>()),
        ("AskRollback", schema_for::<AskRollback>()),
        ("AskUnchanged", schema_for::<AskUnchanged>()),
        ("AskSecret", schema_for::<AskSecret>()),
        ("AskTrustHost", schema_for::<AskTrustHost>()),
        ("SecretAnswer", schema_for::<SecretAnswer>()),
    ]
}

/// The schema of one type, without the file-level keys.
fn schema_for<T: JsonSchema>() -> Value {
    SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
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

/// The schema as the text of `protocol.schema.json`.
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
