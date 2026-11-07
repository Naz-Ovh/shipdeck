//! The agent on a server: which build it needs, where it is installed, and
//! the channel the engine keeps open while the session lives.

mod binary;
mod install;
mod session;

pub use binary::target_for;
pub use session::Agent;
