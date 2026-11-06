//! The messages between `shipdeck engine` and its apps: JSON-RPC 2.0, one
//! message per line of UTF-8. Frontends depend on this crate and nothing
//! else of shipdeck, so the two sides cannot drift apart.
//!
//! The crate holds only the words: no engine logic, no SSH, no file access.
//! [`schema`] writes the same words as JSON Schema, and [`connection`] sends
//! and receives them over any pair of streams.

pub mod connection;
pub mod message;
pub mod schema;
pub mod wire;

/// The protocol version this crate speaks.
pub const VERSION: u32 = 1;
