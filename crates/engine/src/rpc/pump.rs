//! Everything the engine reports, written onto the wire: a running
//! operation's steps and logs, and the questions both ask and wait for.

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::mpsc::UnboundedReceiver;

use shipdeck_protocol::connection::Handle;
use shipdeck_protocol::message::ask::{
    AskRollback, AskSecret, AskTrustHost, AskUnchanged, SecretAnswer,
};
use shipdeck_protocol::message::event::{OpFinished, OpLog, OpProgress, OpRelease, OpStep};
use shipdeck_protocol::message::method;

use crate::deploy::report::{OpEvent, OpId, OpUpdate, RollbackQuestion, UnchangedQuestion};
use crate::ssh::Question;

use super::convert;
use super::session::Session;

/// Forwards each SSH question as the message the app answers.
///
/// Each one gets a task of its own, so a question nobody answers yet does not
/// hold up the next one.
pub fn questions(connection: Handle, mut inbox: UnboundedReceiver<Question>) {
    tokio::spawn(async move {
        while let Some(question) = inbox.recv().await {
            let connection = connection.clone();
            tokio::spawn(async move { ask(&connection, question).await });
        }
    });
}

async fn ask(connection: &Handle, question: Question) {
    match question {
        Question::Secret {
            title,
            prompt,
            reply,
        } => {
            let asked = AskSecret { title, prompt };
            if let Ok(answer) = connection
                .request::<_, SecretAnswer>(method::ASK_SECRET, &asked)
                .await
            {
                let _ = reply.send(answer.secret);
            }
        }
        Question::TrustHost {
            host,
            algorithm,
            fingerprint,
            reply,
        } => {
            let asked = AskTrustHost {
                host,
                algorithm,
                fingerprint,
            };
            if let Ok(trusted) = connection
                .request::<_, bool>(method::ASK_TRUST_HOST, &asked)
                .await
            {
                let _ = reply.send(trusted);
            }
        }
    }
}

/// Forwards one operation's updates until it ends, then frees the engine.
pub fn operation(
    connection: Handle,
    session: Arc<Session>,
    op: OpId,
    mut inbox: UnboundedReceiver<OpUpdate>,
) {
    tokio::spawn(async move {
        while let Some(update) = inbox.recv().await {
            forward(&connection, update.op, update.event);
        }
        session.end(op);
    });
}

/// Writes one update as the message that carries it.
fn forward(connection: &Handle, op: OpId, event: OpEvent) {
    match event {
        OpEvent::Step {
            step,
            state,
            detail,
        } => report(
            connection,
            method::OP_STEP,
            &OpStep {
                op,
                step: convert::index(step),
                state: convert::step_state(state),
                detail,
            },
        ),
        OpEvent::Progress {
            step,
            done,
            total,
            unit,
            label,
        } => report(
            connection,
            method::OP_PROGRESS,
            &OpProgress {
                op,
                step: convert::index(step),
                done,
                total,
                unit: convert::unit(unit),
                label,
            },
        ),
        OpEvent::Log { kind, text } => report(
            connection,
            method::OP_LOG,
            &OpLog {
                op,
                kind: convert::log_kind(kind),
                text,
            },
        ),
        OpEvent::Release(release) => {
            report(connection, method::OP_RELEASE, &OpRelease { op, release });
        }
        OpEvent::Finished(outcome) => report(
            connection,
            method::OP_FINISHED,
            &OpFinished {
                op,
                outcome: convert::outcome(&outcome),
            },
        ),
        OpEvent::AskRollback(question) => {
            let connection = connection.clone();
            tokio::spawn(async move { ask_rollback(&connection, op, question).await });
        }
        OpEvent::AskUnchanged(question) => {
            let connection = connection.clone();
            tokio::spawn(async move { ask_unchanged(&connection, op, question).await });
        }
    }
}

/// One message with no answer expected. The app may be gone; then there is
/// nobody left to tell.
fn report<P: Serialize>(connection: &Handle, method: &str, params: &P) {
    let _ = connection.notify(method, params);
}

/// Asks whether to roll back. With no answer, the deploy rolls back on its
/// own once its seconds are up.
async fn ask_rollback(connection: &Handle, op: OpId, question: RollbackQuestion) {
    let asked = AskRollback {
        op,
        release: question.release,
        previous: question.previous,
        step: question.step.to_string(),
        error: question.error,
        seconds: question.seconds,
    };
    if let Ok(roll_back) = connection
        .request::<_, bool>(method::ASK_ROLLBACK, &asked)
        .await
    {
        let _ = question.reply.send(roll_back);
    }
}

/// Asks whether to deploy a release the live one already holds. With no
/// answer, nothing is deployed.
async fn ask_unchanged(connection: &Handle, op: OpId, question: UnchangedQuestion) {
    let asked = AskUnchanged {
        op,
        release: question.release,
        live: question.live,
    };
    if let Ok(deploy) = connection
        .request::<_, bool>(method::ASK_UNCHANGED, &asked)
        .await
    {
        let _ = question.reply.send(deploy);
    }
}

#[cfg(test)]
#[path = "pump_test.rs"]
mod tests;
