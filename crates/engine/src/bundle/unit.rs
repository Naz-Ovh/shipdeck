//! The systemd unit each release carries, rendered from the built-in
//! template or the deployable's `service.template`.

use anyhow::{Context, Result, bail};

use super::ProcessSpec;
use crate::config::{Deployable, Program, Service, release_path};
use crate::release::ReleaseName;

const DEFAULT_TEMPLATE: &str = include_str!("default.service");

/// The placeholders a template may use.
pub const PLACEHOLDERS: [&str; 12] = [
    "name", "tag", "id", "version", "git", "dir", "service", "user", "exec", "binary", "env",
    "workdir",
];

/// Renders the unit. `start_path` is where preflight found the start
/// program on the server, when `service.start` names one like `node`.
///
/// # Errors
///
/// Fails when the custom template cannot be read, or a placeholder is
/// unknown or has no value.
pub fn render_for(
    deployable: &Deployable,
    release: &ReleaseName,
    start_path: Option<&str>,
) -> Result<String> {
    render_in(deployable, release, start_path, "")
}

/// Renders a service for a folder within the shared release.
pub(super) fn render_in(
    deployable: &Deployable,
    release: &ReleaseName,
    start_path: Option<&str>,
    release_dir: &str,
) -> Result<String> {
    let binary_name = deployable.binary_name();
    let template = deployable.unit_template_path();
    let spec = ProcessSpec {
        name: &deployable.config.name,
        service: &deployable.config.service,
        binary_name: binary_name.as_deref(),
        user: deployable.run_as(),
        template: Some(&template),
        release_dir,
    };
    render_spec(&spec, release, start_path)
}

/// Renders a service using the supplied process settings.
pub(super) fn render_spec(
    spec: &ProcessSpec<'_>,
    release: &ReleaseName,
    start_path: Option<&str>,
) -> Result<String> {
    let custom = spec.template.filter(|path| path.is_file());
    let template = if let Some(custom) = custom {
        std::fs::read_to_string(custom).with_context(|| format!("reading {}", custom.display()))?
    } else {
        DEFAULT_TEMPLATE.to_string()
    };
    let service = spec.service;
    let cwd = workdir(service, spec.release_dir);
    let env_file = service
        .env_file
        .as_ref()
        .map(|file| format!("EnvironmentFile={cwd}/{}", release_path(file)));
    let env = env_file
        .into_iter()
        .chain(
            service
                .env
                .iter()
                .map(|(key, value)| format!("Environment=\"{key}={value}\"")),
        )
        .collect::<Vec<_>>()
        .join("\n");
    let values = [
        Some(spec.name.to_string()),
        Some(release.tag()),
        Some(release.id()),
        Some(release.version.clone()),
        Some(release.git.clone()),
        Some(service.dir.clone()),
        Some(service.name.clone()),
        Some(spec.user.to_string()),
        Some(exec_start_service(
            service,
            spec.binary_name,
            start_path,
            spec.release_dir,
        )),
        spec.binary_name.map(str::to_string),
        Some(env),
        Some(cwd),
    ];
    let vars: Vec<(&str, Option<String>)> = PLACEHOLDERS.iter().copied().zip(values).collect();
    render(&template, &vars).with_context(|| {
        if let Some(custom) = custom {
            format!("rendering {}", custom.display())
        } else {
            "rendering the built-in unit".to_string()
        }
    })
}

/// The `ExecStart=` command: `service.start` (or the binary) with its
/// program made absolute.
#[must_use]
pub fn exec_start(deployable: &Deployable, start_path: Option<&str>) -> String {
    exec_start_in(deployable, start_path, "")
}

/// The start command for a folder within the shared release.
fn exec_start_in(deployable: &Deployable, start_path: Option<&str>, release_dir: &str) -> String {
    exec_start_service(
        &deployable.config.service,
        deployable.binary_name().as_deref(),
        start_path,
        release_dir,
    )
}

/// Builds the start command from process settings.
fn exec_start_service(
    service: &Service,
    binary: Option<&str>,
    start_path: Option<&str>,
    release_dir: &str,
) -> String {
    let (program, rest) = service.command(binary);
    [
        program_path_for(service, program, start_path, release_dir),
        rest,
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

/// Where the start program is on the server.
#[must_use]
pub fn program_path(deployable: &Deployable, program: Program, start_path: Option<&str>) -> String {
    program_path_for(&deployable.config.service, program, start_path, "")
}

/// Where an app's start program is within the shared release.
pub(super) fn program_path_for(
    service: &Service,
    program: Program,
    start_path: Option<&str>,
    release_dir: &str,
) -> String {
    match program {
        Program::Absolute(path) => path,
        Program::InRelease(path) => format!("{}/{path}", workdir(service, release_dir)),
        // systemd finds a bare name on its own fixed PATH, which may not be
        // where preflight found it; the found path is the one to use.
        Program::Tool(name) => start_path.map_or(name, str::to_string),
    }
}

/// The live working folder for a service.
pub(super) fn workdir(service: &Service, release_dir: &str) -> String {
    if release_dir.is_empty() {
        format!("{}/current", service.dir)
    } else {
        format!("{}/current/{release_dir}", service.dir)
    }
}

/// Replaces every `{{name}}`; an unknown placeholder, or one this deployable
/// has no value for, is an error rather than silent text.
///
/// # Errors
///
/// Fails when a `{{` is never closed, or a placeholder is unknown or has
/// no value.
pub fn render(template: &str, vars: &[(&str, Option<String>)]) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            bail!("a `{{{{` is never closed");
        };
        let key = after[..end].trim();
        match vars.iter().find(|(name, _)| *name == key) {
            Some((_, Some(value))) => out.push_str(value),
            Some((_, None)) => bail!(
                "{{{{{key}}}}} has no value: this deployable has no pipeline.pack.binary; use {{{{exec}}}}"
            ),
            None => bail!(
                "unknown placeholder {{{{{key}}}}}; known: {}",
                PLACEHOLDERS.join(", ")
            ),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}
