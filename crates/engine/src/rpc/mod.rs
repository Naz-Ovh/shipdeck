//! `shipdeck engine`: the process apps talk to over stdin and stdout.
//!
//! One message per line comes in; the method it names does the work, and the
//! answer goes back the same way. A deploy runs in a task of its own and
//! reports through [`pump`], and the SSH questions reach the app the same way.

mod convert;
mod logs;
mod operations;
mod pump;
mod releases;
mod run;
mod server_setup;
mod service;
mod session;
mod work;
mod workspace;

pub use run::run;
