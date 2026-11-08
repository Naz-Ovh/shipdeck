//! `work/get` and `work/cleanup`: what an interrupted operation left, and
//! removing it only after the user approved the exact paths.

use serde_json::Value;

use shipdeck_protocol::message::request::{
    EventsRead, EventsReadResult, WorkCleanup, WorkCleanupResult, WorkGet, WorkReport,
};
use shipdeck_protocol::wire::Id;

use super::convert;
use super::session::Session;

/// Reads the live release, unfinished files, recent events, and an
/// activation whose result was never confirmed.
pub async fn get(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: WorkGet = convert::params(params)?;
        let deployable = session.deployment(&asked.deployable)?;
        let servers = session.servers(&deployable).await?;
        let server = servers
            .first()
            .ok_or_else(|| convert::failed(&anyhow::anyhow!("no apps")))?;
        let agent = agent(server)?;
        let inspection = agent
            .inspect_work(server.dir())
            .await
            .map_err(|err| convert::failed(&err))?;
        let events = agent
            .recent_events(server.dir(), 50)
            .await
            .map_err(|err| convert::failed(&err))?
            .events;
        let active = server.status().await.ok().map(|status| status.active);
        convert::value(&WorkReport {
            current: inspection.current,
            active,
            unfinished: inspection.unfinished,
            uncertain: inspection.uncertain,
            events,
        })
    }
    .await;
    session.reply(id, reply);
}

/// Removes exactly the unfinished paths the user approved.
pub async fn cleanup(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: WorkCleanup = convert::params(params)?;
        if asked.artifacts.is_empty() {
            return convert::value(&WorkCleanupResult {
                removed: Vec::new(),
            });
        }
        let deployable = session.deployment(&asked.deployable)?;
        let servers = session.servers(&deployable).await?;
        let server = servers
            .first()
            .ok_or_else(|| convert::failed(&anyhow::anyhow!("no apps")))?;
        let agent = agent(server)?;
        // Hold the project lock for the whole check-and-remove, so a deploy
        // cannot change the release between the inspection and the deletion.
        let guard = crate::agent::ProjectGuard::acquire(server)
            .await
            .map_err(|err| convert::failed(&err))?;
        // Delete only paths the agent itself reports as unfinished now.
        let inspection = agent
            .inspect_work(server.dir())
            .await
            .map_err(|err| convert::failed(&err))?;
        if let Some(unexpected) = asked
            .artifacts
            .iter()
            .find(|path| !inspection.unfinished.contains(path))
        {
            return Err(convert::failed(&anyhow::anyhow!(
                "{unexpected} is not one of the unfinished files the server reported"
            )));
        }
        let removed = agent
            .cleanup_work(server.dir(), &asked.artifacts)
            .await
            .map_err(|err| convert::failed(&err))?;
        guard.release().await.map_err(|err| convert::failed(&err))?;
        convert::value(&WorkCleanupResult { removed })
    }
    .await;
    session.reply(id, reply);
}

/// Reads a page of saved events for one project.
pub async fn events(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: EventsRead = convert::params(params)?;
        let deployable = session.deployment(&asked.deployable)?;
        let servers = session.servers(&deployable).await?;
        let server = servers
            .first()
            .ok_or_else(|| convert::failed(&anyhow::anyhow!("no apps")))?;
        let agent = agent(server)?;
        let limit = asked.limit.clamp(1, 1000);
        let events = agent
            .events_after(Some(server.dir()), asked.sequence, limit)
            .await
            .map_err(|err| convert::failed(&err))?;
        convert::value(&EventsReadResult {
            events: events.events,
        })
    }
    .await;
    session.reply(id, reply);
}

fn agent(
    server: &crate::remote::Server,
) -> Result<std::sync::Arc<crate::agent::Agent>, shipdeck_protocol::wire::ErrorObject> {
    server.session().agent().ok_or_else(|| {
        convert::failed(&anyhow::anyhow!(
            "this connection has no running agent; reconnect so shipdeck can prepare it"
        ))
    })
}
