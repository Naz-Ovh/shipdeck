//! `server/setup`: guided one-time preparation of a server.

use serde_json::Value;

use shipdeck_protocol::message::request::ServerSetup;
use shipdeck_protocol::wire::Id;

use crate::config::Deployment;
use crate::setup::SetupProcess;

use super::convert;
use super::session::Session;

/// Prepares the server once: packages, the project folder, and sudo access.
pub async fn run(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: ServerSetup = convert::params(params)?;
        let deployment = session.deployment(&asked.deployable)?;
        let pool = session.pool()?;
        let connection = pool
            .get(&deployment.endpoint)
            .await
            .map_err(|err| convert::failed(&err))?;
        let processes = processes(&deployment);
        let dir = deployment.dir().to_string();
        let report = crate::setup::guided(
            &dir,
            &processes,
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

fn processes(deployment: &Deployment) -> Vec<SetupProcess> {
    deployment
        .process_apps()
        .map(|(name, manager)| SetupProcess {
            name: deployment.process_name(name),
            manager: Some(manager.kind()),
            user: deployment.run_user(name),
        })
        .collect()
}
