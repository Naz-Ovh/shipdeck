//! A command: text for `sh`, or one runner and its code.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// A command: text run with `sh`, or one runner and its code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Command {
    /// A line for `sh -c`.
    Sh(String),
    /// One runner and the code it runs.
    Runner {
        /// Which runner runs the code.
        runner: Runner,
        /// The code, as written.
        code: String,
    },
}

/// A program that runs a command's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Runner {
    /// `sh -c`.
    Sh,
    /// `bash -c`.
    Bash,
    /// `zsh -c`.
    Zsh,
    /// `node -e`.
    Node,
    /// `python3 -c`.
    Python3,
}

impl Runner {
    pub const ALL: [Runner; 5] = [
        Runner::Sh,
        Runner::Bash,
        Runner::Zsh,
        Runner::Node,
        Runner::Python3,
    ];

    /// The key a deploy file names it by.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Sh => "sh",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Node => "node",
            Self::Python3 => "python3",
        }
    }

    /// The flag that runs code from a string.
    #[must_use]
    pub fn flag(self) -> &'static str {
        match self {
            Self::Sh | Self::Bash | Self::Zsh | Self::Python3 => "-c",
            Self::Node => "-e",
        }
    }

    /// The runner a deploy file key names, if any.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|runner| runner.key() == key)
    }
}

impl<'de> Deserialize<'de> for Command {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CommandVisitor;
        impl<'de> Visitor<'de> for CommandVisitor {
            type Value = Command;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a command, or one runner (`sh`, `bash`, `zsh`, `node`, `python3`) and its code")
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<Command, E> {
                Ok(Command::Sh(text.to_string()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Command, A::Error> {
                let Some(key) = map.next_key::<String>()? else {
                    return Err(de::Error::custom(quotes_help("an empty map")));
                };
                let code = map.next_value::<String>()?;
                if map.next_key::<String>()?.is_some() {
                    return Err(de::Error::custom(quotes_help(&format!(
                        "a map with {key:?} and more"
                    ))));
                }
                match Runner::from_key(&key) {
                    Some(runner) => Ok(Command::Runner { runner, code }),
                    None => Err(de::Error::custom(quotes_help(&format!(
                        "a map with the key {key:?}"
                    )))),
                }
            }
        }
        deserializer.deserialize_any(CommandVisitor)
    }
}

/// The hint an unquoted `a: b` command needs.
fn quotes_help(what: &str) -> String {
    format!(
        "{what} reads as a command map; a command is text, or one runner and its code. \
         Put the whole command in quotes if it holds `: `"
    )
}

impl JsonSchema for Command {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Command".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "title": "Command",
            "description": "A command: a line run with `sh`, or one runner and the code it runs.",
            "anyOf": [
                { "type": "string" },
                { "type": "object" }
            ]
        })
    }
}

/// One command or a list of them, run in order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommandList(pub Vec<Command>);

impl CommandList {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.0.iter()
    }
}

impl<'de> Deserialize<'de> for CommandList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OneOrMany {
            One(Command),
            Many(Vec<Command>),
        }
        Ok(Self(match OneOrMany::deserialize(deserializer)? {
            OneOrMany::One(command) => vec![command],
            OneOrMany::Many(commands) => commands,
        }))
    }
}

impl JsonSchema for CommandList {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CommandList".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let command = Command::json_schema(generator);
        json_schema!({
            "description": "One command, or a list of them run in order, stopping at the first failure.",
            "anyOf": [
                command.clone(),
                { "type": "array", "items": command }
            ]
        })
    }
}
