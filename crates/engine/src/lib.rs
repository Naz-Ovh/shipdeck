//! The deploy engine: reading the config, building, packing, talking to
//! the server over SSH, and the deploy and rollback flows. The apps only
//! start a deploy and show what it reports.
//!
//! [`rpc`] serves those reports to an app over the protocol's pipe.

pub mod agent;
pub mod build;
pub mod bundle;
pub mod config;
pub mod deploy;
pub mod human;
pub mod init;
pub mod release;
pub mod remote;
pub mod rpc;
pub mod ssh;
pub mod tree;
