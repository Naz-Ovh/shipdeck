//! The deploy engine: reading `deploy.json`, building, packing, talking to
//! the server over SSH, and the deploy and rollback flows. The TUI in
//! `bin/shipdeck` only shows what this crate reports.

pub mod build;
pub mod bundle;
pub mod config;
pub mod deploy;
pub mod human;
pub mod release;
pub mod remote;
pub mod setup;
pub mod ssh;
