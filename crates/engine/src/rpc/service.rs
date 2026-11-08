//! `status/get`, `connection/close` and `service/control`: the service on
//! one server, looked at or told what to do.

use serde_json::Value;

use shipdeck_protocol::message::request::{
    ConnectionClose, Done, ServerInfoGet, ServiceControl, StatusGet,
};
use shipdeck_protocol::wire::Id;

use super::convert;
use super::session::Session;

/// Reads facts from the agent on a deployable's server.
pub async fn server_info(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: ServerInfoGet = convert::params(params)?;
        let deployable = session.deployment(&asked.deployable)?;
        let servers = session.servers(&deployable).await?;
        let server = servers
            .first()
            .ok_or_else(|| convert::failed(&anyhow::anyhow!("no apps")))?;
        let agent = server.session().agent().ok_or_else(|| {
            convert::failed(&anyhow::anyhow!("the server agent is not available"))
        })?;
        let facts = agent
            .server_facts(server.dir())
            .await
            .map_err(|err| convert::failed(&err))?;
        convert::value(&facts)
    }
    .await;
    session.reply(id, reply);
}

/// Reads the service's state: its folder, the release `current` points to,
/// and whether it is running.
pub async fn status(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: StatusGet = convert::params(params)?;
        let deployable = session.deployment(&asked.deployable)?;
        let servers = session.servers(&deployable).await?;
        let server = servers
            .first()
            .ok_or_else(|| convert::failed(&anyhow::anyhow!("no apps")))?;
        let mut state = server.status().await.map_err(|err| convert::failed(&err))?;
        if servers.len() > 1 {
            let mut states = Vec::with_capacity(servers.len());
            for server in &servers {
                states.push((
                    server.process_name().to_string(),
                    server
                        .status()
                        .await
                        .map_err(|err| convert::failed(&err))?
                        .active,
                ));
            }
            state.active = if states.iter().all(|(_, state)| state == "active") {
                "active".to_string()
            } else {
                format!(
                    "apps: {}",
                    states
                        .iter()
                        .map(|(name, state)| format!("{name}: {state}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
        }
        convert::value(&convert::status(&state))
    }
    .await;
    session.reply(id, reply);
}

/// Drops this project's server connection. Every project on the same server
/// shares it, so they are all dropped.
pub async fn close(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: ConnectionClose = convert::params(params)?;
        let deployable = session.deployment(&asked.deployable)?;
        let pool = session.pool()?;
        pool.close(deployable.endpoint()).await;
        convert::value(&Done {})
    }
    .await;
    session.reply(id, reply);
}

/// Stops, starts or restarts the service.
pub async fn control(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: ServiceControl = convert::params(params)?;
        let deployable = session.deployment(&asked.deployable)?;
        let servers = session.servers(&deployable).await?;
        let first = servers
            .first()
            .ok_or_else(|| convert::failed(&anyhow::anyhow!("no apps")))?;
        let guard = crate::agent::ProjectGuard::acquire(first)
            .await
            .map_err(|err| convert::failed(&err))?;
        for server in &servers {
            server
                .control(convert::control(asked.action))
                .await
                .map_err(|err| convert::failed(&err))?;
        }
        guard.release().await.map_err(|err| convert::failed(&err))?;
        convert::value(&Done {})
    }
    .await;
    session.reply(id, reply);
}
