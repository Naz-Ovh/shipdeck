//! Assembles all app files and process files into one shared release.

use std::path::Path;

use anyhow::{Context, Result};

use super::{AppSource, Contents, ProcessSpec, ServiceFile, gather_apps, service_file_for};
use crate::config::monorepo::{MonorepoConfig, ResolvedApp};
use crate::config::{Program, UNIT_TEMPLATE};
use crate::release::ReleaseName;
use crate::remote::Facts;

/// Collects every app and its process file into one release.
///
/// # Errors
/// Returns an error if a process file or an app's files cannot be collected.
pub fn gather_monorepo(
    config: &MonorepoConfig,
    folder: &Path,
    apps: &[ResolvedApp],
    user: &str,
    release: &ReleaseName,
    facts: &Facts,
    progress: impl FnMut(u64, u64),
) -> Result<Contents> {
    let files = process_files(config, folder, apps, user, release, facts)?;
    let sources: Vec<_> = apps
        .iter()
        .zip(&files)
        .map(|(app, file)| AppSource {
            project: &app.project,
            deploy_folder: folder,
            pack: &app.pipeline.pack,
            service: &app.service,
            service_file: file.as_ref(),
            release_dir: &app.release_dir,
            excluded_configs: &config.app_files,
        })
        .collect();
    let metadata = config.release_metadata()?;
    gather_apps(&sources, &metadata, progress)
}

/// Renders every app's manager file for packing or stable fingerprinting.
fn process_files(
    config: &MonorepoConfig,
    folder: &Path,
    apps: &[ResolvedApp],
    user: &str,
    release: &ReleaseName,
    facts: &Facts,
) -> Result<Vec<Option<ServiceFile>>> {
    let template = folder.join(UNIT_TEMPLATE);
    let mut files: Vec<Option<ServiceFile>> = Vec::with_capacity(apps.len());
    for app in apps {
        if app.service.is_static() {
            files.push(None);
            continue;
        }
        let binary = app
            .pipeline
            .pack
            .binary
            .as_deref()
            .and_then(|path| Path::new(path).file_name().and_then(|name| name.to_str()));
        let (program, _) = app.service.command(binary);
        let start_path = match &program {
            Program::Tool(tool) => facts.tool_path(tool),
            _ => None,
        };
        let spec = ProcessSpec {
            name: &config.name,
            service: &app.service,
            binary_name: binary,
            user: app.service.run_as.as_deref().unwrap_or(user),
            template: Some(&template),
            release_dir: &app.release_dir,
        };
        files.push(
            service_file_for(&spec, release, start_path)
                .with_context(|| format!("rendering the process for app {}", app.name))?,
        );
    }
    Ok(files)
}

/// Fingerprints process files with a fixed release name, including custom templates.
///
/// # Errors
/// Returns an error when a process file cannot be rendered.
pub fn fingerprint_monorepo(
    contents: &Contents,
    config: &MonorepoConfig,
    folder: &Path,
    apps: &[ResolvedApp],
    user: &str,
    facts: &Facts,
) -> Result<String> {
    let unnamed = ReleaseName::new("0.0.0", "unnamed", false, chrono::DateTime::UNIX_EPOCH);
    let files = process_files(config, folder, apps, user, &unnamed, facts)?;
    let files: Vec<_> = files.iter().filter_map(Option::as_ref).collect();
    Ok(contents.fingerprint_apps(&files, &serde_json::to_string(config)?))
}

#[cfg(test)]
#[path = "apps_test.rs"]
mod tests;
