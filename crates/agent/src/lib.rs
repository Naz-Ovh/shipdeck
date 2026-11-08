//! `shipdeck-agent`: the small program the engine starts on a server over
//! SSH. It speaks the same JSON-RPC as the app protocol, on stdin and stdout,
//! and exits when the connection ends. It writes nothing else to stdout.

mod backup;
mod cleanup;
mod dispatch;
mod env_file;
mod facts;
mod files;
mod identity;
mod inspect_release;
mod locks;
mod logs;
mod owner;
mod paths;
mod project;
mod releases;
mod serve;
mod service;
mod state;
mod transfer;
mod unpack;
mod unpack_reuse;
mod unpack_verify;

pub use serve::serve;
pub use transfer::receive_recorded;
