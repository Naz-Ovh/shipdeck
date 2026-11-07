//! `shipdeck-agent`: the small program the engine starts on a server over
//! SSH. It speaks the same JSON-RPC as the app protocol, on stdin and stdout,
//! and exits when the connection ends. It writes nothing else to stdout.

mod serve;

pub use serve::serve;
