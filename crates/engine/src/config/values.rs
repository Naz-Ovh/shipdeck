//! The values inside a deploy file: files, commands and TLS.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// `tls: none`, or a Let's Encrypt certificate for every domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Tls {
    /// `none`: plain HTTP only.
    None(TlsNone),
    /// A certificate for every domain.
    LetsEncrypt(TlsProvider),
}

/// The word `none`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsNone {
    /// Plain HTTP only.
    None,
}

/// A certificate provider.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TlsProvider {
    /// The provider. Only `letsencrypt` is known.
    pub provider: String,
    /// Where Let's Encrypt sends its notices.
    pub email: String,
}

impl<'de> Deserialize<'de> for Tls {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TlsVisitor;
        impl<'de> Visitor<'de> for TlsVisitor {
            type Value = Tls;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("`none`, or a map with provider and email")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<Tls, E> {
                if text == "none" {
                    Ok(Tls::None(TlsNone::None))
                } else {
                    Err(E::custom(format!(
                        "`tls` is `none` or a map with `provider` and `email`, not {text:?}"
                    )))
                }
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Tls, A::Error> {
                TlsProvider::deserialize(de::value::MapAccessDeserializer::new(map))
                    .map(Tls::LetsEncrypt)
            }
        }
        deserializer.deserialize_any(TlsVisitor)
    }
}

impl JsonSchema for Tls {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Tls".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "`none` for plain HTTP, or a map with `provider` and `email`.",
            "anyOf": [
                { "type": "string", "const": "none" },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["provider", "email"],
                    "properties": {
                        "provider": { "type": "string", "description": "Only `letsencrypt` is known." },
                        "email": { "type": "string", "description": "Where Let's Encrypt sends its notices." }
                    }
                }
            ]
        })
    }
}

/// A file in a release: a path, or a path and where it lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum FileEntry {
    /// A path on your PC. It lands at its path from the project folder, or
    /// inside the app's source for an app file.
    Path(String),
    /// A path on your PC and where it lands in the release.
    To(MappedFile),
}

/// A path and, when it needs one, its place in the release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MappedFile {
    /// The path on your PC.
    pub path: String,
    /// Where it lands, relative to the release or app folder. Optional for a
    /// binary, whose file name is its place.
    #[serde(default)]
    pub to: Option<String>,
}

impl<'de> Deserialize<'de> for FileEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FileVisitor;
        impl<'de> Visitor<'de> for FileVisitor {
            type Value = FileEntry;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a path, or a map with `path` and `to`")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<FileEntry, E> {
                Ok(FileEntry::Path(text.to_string()))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<FileEntry, A::Error> {
                MappedFile::deserialize(de::value::MapAccessDeserializer::new(map))
                    .map(FileEntry::To)
            }
        }
        deserializer.deserialize_any(FileVisitor)
    }
}

impl JsonSchema for FileEntry {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FileEntry".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mapped = MappedFile::json_schema(generator);
        json_schema!({
            "description": "A path on your PC, or a map with `path` and `to`.",
            "anyOf": [
                { "type": "string" },
                mapped
            ]
        })
    }
}
