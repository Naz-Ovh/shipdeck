//! `deploy/start` and `rollback/start`: the long runs.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::unbounded_channel;

use shipdeck_protocol::message::request::{DeployStart, OperationStarted, RollbackStart};
use shipdeck_protocol::wire::{ErrorObject, Id};

use crate::config::{DeployConfig, Deployment};
use crate::deploy::report::{OpId, Reporter};
use crate::deploy::{self, Step};
use crate::ssh::Pool;

use super::convert;
use super::pump;
use super::session::Session;

/// One long run, once its checks pass.
struct Work {
    deployable: Deployment,
    pool: Arc<Pool>,
    op: OpId,
    steps: Vec<Step>,
}

/// Builds and deploys the project. The answer carries the operation's id and
/// its steps; everything after that comes as `op/*` events.
pub async fn deploy(session: &Arc<Session>, id: Id, params: Value) {
    let prepared = async {
        let asked: DeployStart = convert::params(params)?;
        let mut deployable = session.deployment(&asked.deployable)?;
        if let Some(choice) = asked.on_failure {
            match &mut deployable {
                Deployment::Single(d) => {
                    d.config.pipeline.failed.rollback = convert::on_failure(choice);
                }
                Deployment::Monorepo(d) => {
                    d.config.pipeline["failed"]["rollback"] = serde_json::json!(match choice {
                        shipdeck_protocol::message::common::OnFailure::Rollback => "auto",
                        shipdeck_protocol::message::common::OnFailure::Keep => "ask",
                    });
                }
            }
        }
        ready(session, deployable, deploy::deploy_steps)
    }
    .await;
    let work = match prepared {
        Ok(work) => work,
        Err(error) => return session.reply_error(id, error),
    };
    let reporter = start(session, id, &work);
    match work.deployable {
        Deployment::Single(d) => {
            tokio::spawn(deploy::deploy(*d, work.pool, reporter));
        }
        Deployment::Monorepo(d) => {
            tokio::spawn(deploy::deploy_monorepo(*d, work.pool, reporter));
        }
    }
}

/// Puts one release back. The answer is the same shape as a deploy's.
pub async fn rollback(session: &Arc<Session>, id: Id, params: Value) {
    let prepared = async {
        let asked: RollbackStart = convert::params(params)?;
        let deployable = session.deployment(&asked.deployable)?;
        let work = ready(session, deployable, deploy::rollback_steps)?;
        Ok::<(Work, String), ErrorObject>((work, asked.release))
    }
    .await;
    let (work, target) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return session.reply_error(id, error),
    };
    let reporter = start(session, id, &work);
    match work.deployable {
        Deployment::Single(d) => {
            tokio::spawn(deploy::rollback(*d, target, work.pool, reporter));
        }
        Deployment::Monorepo(d) => {
            tokio::spawn(deploy::rollback_monorepo(*d, target, work.pool, reporter));
        }
    }
}

/// The project, its connections and a free engine, or why the run cannot
/// start.
fn ready(
    session: &Session,
    deployable: Deployment,
    plan: fn(&DeployConfig) -> Vec<Step>,
) -> Result<Work, ErrorObject> {
    let steps = match &deployable {
        Deployment::Single(d) => plan(&d.config),
        Deployment::Monorepo(_) => deploy::monorepo_steps(),
    };
    let pool = session.pool()?;
    let op = session.begin()?;
    Ok(Work {
        deployable,
        pool,
        op,
        steps,
    })
}

/// Answers with the operation before any of its events is written, and
/// returns what the run reports through.
fn start(session: &Arc<Session>, id: Id, work: &Work) -> Reporter {
    let (reports, inbox): (_, UnboundedReceiver<_>) = unbounded_channel();
    pump::operation(session.connection(), Arc::clone(session), work.op, inbox);
    let started = OperationStarted {
        op: work.op,
        steps: convert::steps(&work.steps),
    };
    session.reply(id, convert::value(&started));
    Reporter::new(work.op, reports)
}
