//! Deployment config format 2: the root file, the deploy file, their
//! checks, placeholders and the resolved deploy.

mod check;
mod commands;
pub mod glob;
pub mod load;
mod names;
pub mod paths;
pub mod placeholders;
mod problem;
mod resolve;
mod rules;
pub mod schema;
mod steps;
mod types;
mod values;
mod yaml;

pub use load::{
    Deploy, ROOT_NAMES, Root, full, is_deploy_file, load_deploy, load_in_place, open_all,
    parse_deploy, read as read_deploy,
};
pub use names::{
    count, file_name, is_deploy_name, is_plain_name, is_program, is_safe_dir, is_unit_name,
    split_args,
};
pub use paths::release_path;
pub use placeholders::{AppValues, Placeholder, Scope, Values};
pub use problem::{FileProblems, Problem, Span, render};
pub use resolve::{Deployment, Manager, ManagerKind, Program, VERSION_FILES};
pub use steps::{AppSteps, ReleaseSteps};
pub use types::{
    App, AppBackupStep, AppHealthStep, BackupStep, BuildStep, Command, CommandList, Commands,
    DeployEntry, DeployFile, FileEntry, HealthStep, Keep, MappedFile, Nginx, Oxmgr, Pm2,
    PreflightStep, Process, Release, RollbackMode, RollbackStep, RootFile, Runner, Server,
    SshTarget, Step, Systemd, SystemdOptions, Tls, TlsNone, TlsProvider,
};
pub use yaml::Node;
