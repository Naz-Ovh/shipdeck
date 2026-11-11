//! Deployment config format 2: the project file, the deploy file, their
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

pub use load::{PROJECT_NAMES, Project, load_deploy, parse_deploy, read as read_deploy};
pub use names::{
    count, file_name, is_plain_name, is_program, is_safe_dir, is_unit_name, split_args,
};
pub use paths::release_path;
pub use placeholders::{AppValues, Placeholder, Scope, Values};
pub use problem::{FileProblems, Problem, Span, render};
pub use resolve::{Deployment, Manager, Program, VERSION_FILES};
pub use steps::{AppSteps, ReleaseSteps};
pub use types::{
    App, AppBackupStep, AppHealthStep, BackupStep, BuildStep, Command, CommandList, Commands,
    DeployFile, FileEntry, HealthStep, Keep, MappedFile, Nginx, Oxmgr, Pm2, PreflightStep, Process,
    ProjectFile, Release, RollbackMode, RollbackStep, Runner, Server, SshTarget, Step, Systemd,
    SystemdOptions, Tls, TlsNone, TlsProvider,
};
pub use yaml::Node;
