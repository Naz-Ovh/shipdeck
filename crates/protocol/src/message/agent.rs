//! The words between the engine and `shipdeck-agent`, the helper the engine
//! starts on a server over SSH. Same envelope as the app protocol; these
//! methods carry an `agent/` prefix so the two sets never meet.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The agent protocol version this crate speaks.
pub const PROTOCOL: u32 = 1;

/// `agent/hello`: agree on the version before anything else.
pub const HELLO: &str = "agent/hello";
/// `agent/ping`: prove the agent is still there.
pub const PING: &str = "agent/ping";
/// `agent/shutdown`: the connection is ending; answer, then exit.
pub const SHUTDOWN: &str = "agent/shutdown";

/// `agent/hello`: what the engine says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHello {
    /// The agent protocol the engine speaks.
    pub protocol: u32,
    /// The engine's version, such as `0.7.1`.
    pub version: String,
}

/// What `agent/hello` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHelloResult {
    /// The agent protocol the agent speaks.
    pub protocol: u32,
    /// The agent's version, which must equal the engine's.
    pub version: String,
    /// The server's operating system, such as `linux`.
    pub os: String,
    /// The server's CPU, such as `x86_64`.
    pub arch: String,
}

/// `agent/ping`: nothing to say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentPing {}

/// `agent/shutdown`: nothing to say; the request itself is the message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentShutdown {}
