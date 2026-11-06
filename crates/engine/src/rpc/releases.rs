//! `releases/list`, `release/contents` and `release/read_file`: what is on
//! the server, and what one release holds.

use serde_json::Value;

use shipdeck_protocol::message::request::{
    ReleaseContents, ReleaseReadFile, ReleaseReadFileResult, ReleasesList,
};
use shipdeck_protocol::wire::Id;

use super::convert;
use super::session::Session;

/// Every release on the server, newest first.
pub async fn list(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: ReleasesList = convert::params(params)?;
        let deployable = session.deployable(&asked.deployable)?;
        let server = session.server(&deployable).await?;
        let found = server.list().await.map_err(|err| convert::failed(&err))?;
        convert::value(&convert::listing(&found))
    }
    .await;
    session.reply(id, reply);
}

/// The paths of one release, from its folder or from inside its backup.
pub async fn contents(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: ReleaseContents = convert::params(params)?;
        let deployable = session.deployable(&asked.deployable)?;
        let server = session.server(&deployable).await?;
        let found = server
            .contents(&asked.release, convert::source(asked.source))
            .await
            .map_err(|err| convert::failed(&err))?;
        convert::value(&convert::contents(&found))
    }
    .await;
    session.reply(id, reply);
}

/// The start of one file of a release.
pub async fn read_file(session: &Session, id: Id, params: Value) {
    let reply = async {
        let asked: ReleaseReadFile = convert::params(params)?;
        let deployable = session.deployable(&asked.deployable)?;
        let server = session.server(&deployable).await?;
        let found = server
            .read(&asked.release, convert::source(asked.source), &asked.path)
            .await
            .map_err(|err| convert::failed(&err))?;
        convert::value(&ReleaseReadFileResult {
            text: found.text,
            binary: found.binary,
        })
    }
    .await;
    session.reply(id, reply);
}
