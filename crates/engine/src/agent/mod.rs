//! The agent on a server: which build it needs, where it is installed, and
//! the channel the engine keeps open while the session lives.

mod binary;
mod development;
mod install;
mod layout;
mod lock;
mod ops;
mod session;
mod transfer;
mod version;

pub use binary::target_for;
pub use lock::ProjectGuard;
pub use ops::AgentLogEvent;
pub use session::Agent;
