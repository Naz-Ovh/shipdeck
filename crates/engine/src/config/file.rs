//! `shipdeck.yaml` as written: a service or a static site, and the pipeline.

use schemars::JsonSchema;
use serde::Deserialize;

use super::{DeployConfig, Pipeline, Service, SshTarget, default_project};

/// `shipdeck.yaml` as written: a service, a static site, or neither (an error).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct DeployConfigFile {
    /// The name shipdeck shows, such as `My API (prod)`.
    pub(super) name: String,
    /// Your source folder, relative to this shipdeck.yaml.
    #[serde(default = "default_project")]
    pub(super) project: String,
    /// The server folder that holds `current`, `releases/`, `backups/` and
    /// `shared/`. Absolute, such as `/opt/my-api`.
    pub(super) dir: String,
    /// How to log in to the server.
    pub(super) ssh: SshTarget,
    /// A process on the server: a systemd unit or a PM2 app.
    #[serde(default)]
    pub(super) service: Option<Service>,
    /// Static files on the server, served by nginx. Use this instead of
    /// `service`.
    #[serde(default, rename = "static")]
    pub(super) static_site: Option<StaticSite>,
    /// The steps of a deploy that take settings.
    #[serde(default)]
    pub(super) pipeline: Pipeline,
}

impl DeployConfigFile {
    /// Resolves the one target the file describes.
    pub(super) fn into_config(self) -> Option<DeployConfig> {
        let service = match (self.service, self.static_site) {
            (Some(mut service), None) => {
                service.dir = self.dir;
                service
            }
            (None, Some(site)) => Service::static_site(self.name.clone(), self.dir, site.root),
            (None, None) | (Some(_), Some(_)) => return None,
        };
        Some(DeployConfig {
            name: self.name,
            project: self.project,
            ssh: self.ssh,
            service,
            pipeline: self.pipeline,
        })
    }
}

/// A static site: files in a release, served by a web server.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StaticSite {
    /// Folder inside the release to serve, such as `dist`. Relative, and
    /// not the release root.
    pub root: String,
}
