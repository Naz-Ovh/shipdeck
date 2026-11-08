//! `server/setup`: guided one-time preparation of a server.

use serde_json::Value;

use shipdeck_protocol::message::request::ServerSetup;
use shipdeck_protocol::wire::Id;

use crate::config::monorepo::AppDefinition;
use crate::config::{Deployment, Service};

use super::convert;
use super::session::Session;

/// Prepares the server once: packages, the project folder, and sudo access.
pub async fn run(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: ServerSetup = convert::params(params)?;
        let deployment = session.deployment(&asked.deployable)?;
        let pool = session.pool()?;
        let connection = pool
            .get(deployment.endpoint())
            .await
            .map_err(|err| convert::failed(&err))?;
        let services = services(&deployment)?;
        let dir = deployment_dir(&deployment);
        let report = crate::setup::guided(
            &dir,
            &services,
            &connection,
            asked.access,
            asked.sudo_password.as_deref(),
        )
        .await
        .map_err(|err| convert::failed(&err))?;
        convert::value(&report)
    }
    .await;
    session.reply(id, reply);
}

fn deployment_dir(deployment: &Deployment) -> String {
    match deployment {
        Deployment::Single(deployable) => deployable.config.service.dir.clone(),
        Deployment::Monorepo(monorepo) => monorepo.config.dir.clone(),
    }
}

fn services(deployment: &Deployment) -> Result<Vec<Service>, shipdeck_protocol::wire::ErrorObject> {
    match deployment {
        Deployment::Single(deployable) => Ok(vec![deployable.config.service.clone()]),
        Deployment::Monorepo(monorepo) => {
            monorepo
                .config
                .check_apps()
                .map_err(|err| convert::failed(&err))?;
            let mut services = Vec::with_capacity(monorepo.config.apps.len());
            for (name, definition) in &monorepo.config.apps {
                let AppDefinition::Inline(app) = definition else {
                    return Err(convert::failed(&anyhow::anyhow!("unresolved app {name}")));
                };
                services.push(app.service(&monorepo.config, name));
            }
            Ok(services)
        }
    }
}
