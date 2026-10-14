//! The systemd unit each release carries, rendered from the built-in
//! template or the deployable's `service.template`.

use anyhow::{Context, Result, bail};

use crate::config::{Deployable, Program, bundle_path};
use crate::release::ReleaseName;

const DEFAULT_TEMPLATE: &str = include_str!("default.service");

/// The placeholders a template may use.
pub const PLACEHOLDERS: [&str; 12] = [
    "name", "tag", "id", "version", "git", "dir", "service", "user", "exec", "binary", "args",
    "env",
];

/// Renders the unit. `start_path` is where preflight found the start
/// program on the server, when `remote.start` names one like `node`.
pub fn render_for(
    deployable: &Deployable,
    release: &ReleaseName,
    start_path: Option<&str>,
) -> Result<String> {
    let custom = deployable.unit_template_path();
    let template = if custom.is_file() {
        std::fs::read_to_string(&custom).with_context(|| format!("reading {}", custom.display()))?
    } else {
        DEFAULT_TEMPLATE.to_string()
    };
    let config = &deployable.config;
    let env_file = config.remote.env_file.as_ref().map(|file| {
        format!(
            "EnvironmentFile={}/current/{}",
            config.remote.dir,
            bundle_path(file)
        )
    });
    let env = env_file
        .into_iter()
        .chain(
            config
                .remote
                .env
                .iter()
                .map(|(key, value)| format!("Environment=\"{key}={value}\"")),
        )
        .collect::<Vec<_>>()
        .join("\n");
    let values = [
        Some(config.name.clone()),
        Some(release.tag()),
        Some(release.id()),
        Some(release.version.clone()),
        Some(release.git.clone()),
        Some(config.remote.dir.clone()),
        Some(config.remote.service.clone()),
        Some(deployable.run_as().to_string()),
        Some(exec_start(deployable, start_path)),
        deployable.binary_name(),
        Some(args(&config.remote.args)),
        Some(env),
    ];
    let vars: Vec<(&str, Option<String>)> = PLACEHOLDERS.iter().copied().zip(values).collect();
    render(&template, &vars).with_context(|| {
        if custom.is_file() {
            format!("rendering {}", custom.display())
        } else {
            "rendering the built-in unit".to_string()
        }
    })
}

/// The `ExecStart=` command: `remote.start` (or the binary) with its
/// program made absolute, then `remote.args`.
pub fn exec_start(deployable: &Deployable, start_path: Option<&str>) -> String {
    let (program, rest) = deployable.start();
    let program = match program {
        Program::Absolute(path) => path,
        Program::InRelease(path) => format!("{}/current/{path}", deployable.config.remote.dir),
        // systemd finds a bare name on its own fixed PATH, which may not be
        // where preflight found it; the found path is the one to use.
        Program::Tool(name) => start_path.map(str::to_string).unwrap_or(name),
    };
    [program, rest, args(&deployable.config.remote.args)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.contains(' ') {
                format!("\"{arg}\"")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replaces every `{{name}}`; an unknown placeholder, or one this deployable
/// has no value for, is an error rather than silent text.
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
                "{{{{{key}}}}} has no value: this deployable has no build.binary; use {{{{exec}}}}"
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
