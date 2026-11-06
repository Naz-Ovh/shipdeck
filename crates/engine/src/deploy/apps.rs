//! Shared release operations and process settings for monorepos.

use std::sync::Arc;

use anyhow::{Result, bail};

use crate::config::monorepo::{AppDefinition, MonorepoConfig};
use crate::remote::{Server, Source};
use crate::ssh::Session;

/// Builds process handles without requiring local app source folders.
pub fn servers(config: &MonorepoConfig, session: &Arc<Session>) -> Result<Vec<Server>> {
    config.check_apps()?;
    config
        .apps
        .iter()
        .map(|(name, definition)| {
            let AppDefinition::Inline(app) = definition else {
                bail!("unresolved app {name}")
            };
            Server::for_app(Arc::clone(session), &app.service(config, name), name)
        })
        .collect()
}

/// Reads saved settings and checks that they belong to this deployment folder.
pub async fn saved(server: &Server, id: &str, source: Source) -> Result<MonorepoConfig> {
    let config = server.monorepo_config(id, source).await?;
    if config.dir != server.dir() {
        bail!("release settings refer to a different deployment folder");
    }
    Ok(config)
}

/// Uses live process settings even when the local app list has changed.
pub async fn live_servers(config: &MonorepoConfig, session: Arc<Session>) -> Result<Vec<Server>> {
    let configured = servers(config, &session)?;
    let first = configured
        .first()
        .ok_or_else(|| anyhow::anyhow!("no apps"))?;
    if let Some(id) = first.list().await?.current {
        let live = saved(first, &id, Source::Folder).await?;
        servers(&live, &session)
    } else {
        Ok(configured)
    }
}
