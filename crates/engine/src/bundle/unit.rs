//! The systemd unit each release carries, rendered from the built-in
//! template or the deployable's `service.template`.

use anyhow::{Context, Result, bail};

use crate::config::Deployable;
use crate::release::ReleaseName;

const DEFAULT_TEMPLATE: &str = include_str!("default.service");

/// The placeholders a template may use.
pub const PLACEHOLDERS: [&str; 11] = [
    "name", "tag", "id", "version", "git", "dir", "service", "user", "binary", "args", "env",
];

pub fn render_for(deployable: &Deployable, release: &ReleaseName) -> Result<String> {
    let custom = deployable.unit_template_path();
    let template = if custom.is_file() {
        std::fs::read_to_string(&custom).with_context(|| format!("reading {}", custom.display()))?
    } else {
        DEFAULT_TEMPLATE.to_string()
    };
    let config = &deployable.config;
    let args = config
        .remote
        .args
        .iter()
        .map(|arg| {
            if arg.contains(' ') {
                format!("\"{arg}\"")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let env = config
        .remote
        .env
        .iter()
        .map(|(key, value)| format!("Environment=\"{key}={value}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let values = [
        config.name.clone(),
        release.tag(),
        release.id(),
        release.version.clone(),
        release.git.clone(),
        config.remote.dir.clone(),
        config.remote.service.clone(),
        deployable.run_as().to_string(),
        deployable.binary_name(),
        args,
        env,
    ];
    let vars: Vec<(&str, String)> = PLACEHOLDERS.iter().copied().zip(values).collect();
    render(&template, &vars).with_context(|| {
        if custom.is_file() {
            format!("rendering {}", custom.display())
        } else {
            "rendering the built-in unit".to_string()
        }
    })
}

/// Replaces every `{{name}}`; an unknown placeholder is an error, not silent text.
pub fn render(template: &str, vars: &[(&str, String)]) -> Result<String> {
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
            Some((_, value)) => out.push_str(value),
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
