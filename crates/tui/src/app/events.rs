//! How results from the engine change the app: fetches, operation events and
//! the engine's questions.

use std::time::{Duration, Instant};

use super::{
    App, Asked, Fetch, Fetched, LOG_LIMIT, Live, LogsState, Modal, OpId, OpKind, Operation, Screen,
    Tone,
};
use shipdeck_engine::deploy::report::{OpEvent, OpUpdate};
use shipdeck_engine::ssh::Question;
use shipdeck_protocol::message::request::SetupReport;

impl App {
    pub fn on_fetched(&mut self, fetched: Fetched) {
        match fetched {
            Fetched::Status { index, result } => {
                if let Some(slot) = self.live.get_mut(index) {
                    *slot = match result {
                        Ok(status) => Live::Ready(status),
                        Err(err) => Live::Error(err),
                    };
                }
            }
            Fetched::Releases { index, result } => {
                if let Some(view) = self.releases.as_mut().filter(|v| v.deployable == index) {
                    view.listing = match result {
                        Ok(listing) => {
                            let rows = listing.entries.len();
                            let selected = view.table.selected().unwrap_or(0);
                            view.table
                                .select((rows > 0).then(|| selected.min(rows - 1)));
                            Fetch::Ready(listing)
                        }
                        Err(err) => Fetch::Error(err),
                    };
                }
            }
            Fetched::Contents {
                index,
                id,
                source,
                result,
            } => self.on_contents(index, &id, source, result),
            Fetched::FileText {
                index,
                id,
                path,
                result,
            } => self.on_file_text(index, &id, &path, result),
            Fetched::Controlled {
                index,
                action,
                result,
            } => self.controlled(index, action, result),
            Fetched::Started {
                index,
                kind,
                result,
            } => self.started(index, kind, result),
            Fetched::LogsFailed { result } => {
                if let Some(logs) = self.logs.as_mut() {
                    logs.state = LogsState::Ended(result);
                }
            }
            Fetched::LogsOpen { view } => match self.logs.as_mut().filter(|l| l.id.is_none()) {
                Some(logs) => {
                    logs.id = Some(view);
                    logs.state = LogsState::Following;
                }
                // The view closed before the engine answered; stop the stream
                // it has just opened.
                None => self.stop_log(view),
            },
            Fetched::LogLine { view, line } => {
                if let Some(logs) = self.logs.as_mut() {
                    // A line can beat the answer that carries its view.
                    if logs.id.is_none() {
                        logs.id = Some(view);
                    }
                    if logs.id != Some(view) {
                        return;
                    }
                    logs.lines.push_back(line);
                    if logs.lines.len() > LOG_LIMIT {
                        logs.lines.pop_front();
                    }
                    // Keep a scrolled-up view still while new lines arrive.
                    if logs.scroll > 0 {
                        logs.scroll = (logs.scroll + 1).min(logs.lines.len());
                    }
                }
            }
            Fetched::LogsEnded { view, result } => {
                if let Some(logs) = self.logs.as_mut().filter(|l| l.id == Some(view)) {
                    logs.state = LogsState::Ended(match result {
                        Ok(Some(code)) => format!("the log ended (exit {code})"),
                        Ok(None) => "the log ended".to_string(),
                        Err(err) => err,
                    });
                }
            }
            Fetched::Facts { index, result } => {
                if let Some(slot) = self.facts.get_mut(index) {
                    *slot = result.ok();
                }
            }
            Fetched::Work { index, result } => {
                if let Some(slot) = self.work.get_mut(index) {
                    *slot = result.ok();
                }
            }
            Fetched::Cleaned { index, result } => self.on_cleaned(index, result),
            Fetched::SetupDone { index, result } => self.on_setup_done(index, result),
        }
    }

    /// The engine accepted a deploy or rollback: from here its events belong
    /// to an operation the screen knows.
    fn started(&mut self, index: usize, kind: OpKind, result: Result<(OpId, Vec<String>), String>) {
        self.starting = false;
        match result {
            Ok((id, steps)) => {
                self.op = Some(Operation::new(id, kind, index, &steps));
                self.screen = Screen::Operation;
                // Events that beat the answer are shown now, in order.
                for update in std::mem::take(&mut self.early) {
                    self.on_op(update);
                }
            }
            Err(err) => {
                self.early.clear();
                self.modals.push_back(Modal::Notice {
                    title: "Could not start".to_string(),
                    text: err,
                });
            }
        }
    }

    pub fn on_op(&mut self, update: OpUpdate) {
        // The answer to `deploy/start` may still be on its way; nothing here
        // can be placed until it says which operation this is.
        if self.starting && self.op.is_none() {
            self.early.push(update);
            return;
        }
        let Some(op) = self.op.as_mut().filter(|op| op.id == update.op) else {
            return;
        };
        let finished = matches!(update.event, OpEvent::Finished(_));
        let index = op.deployable;
        match op.apply(update.event) {
            Some(Asked::Rollback(question)) => {
                let deadline = Instant::now() + Duration::from_secs(question.seconds);
                // In front of everything else: the countdown is already running.
                self.modals.push_front(Modal::AskRollback {
                    op: update.op,
                    deployable: index,
                    question,
                    deadline,
                });
                self.screen = Screen::Operation;
            }
            Some(Asked::Unchanged(question)) => {
                // The deploy waits for it.
                self.modals.push_front(Modal::AskUnchanged {
                    op: update.op,
                    deployable: index,
                    question,
                });
                self.screen = Screen::Operation;
            }
            None => {}
        }
        if finished {
            let op_id = update.op;
            self.modals.retain(|modal| {
                !matches!(modal, Modal::AskRollback { op, .. } | Modal::AskUnchanged { op, .. } if *op == op_id)
            });
            self.refresh_status(index);
            if self
                .releases
                .as_ref()
                .is_some_and(|v| v.deployable == index)
            {
                self.fetch_releases(index);
            }
        }
    }

    fn on_cleaned(&mut self, index: usize, result: Result<Vec<String>, String>) {
        match result {
            Ok(removed) if removed.is_empty() => {
                self.notify("Nothing left to clean up", Tone::Info);
            }
            Ok(removed) => {
                self.notify(
                    format!("Removed {} unfinished file(s)", removed.len()),
                    Tone::Info,
                );
            }
            Err(err) => self.modals.push_back(Modal::Notice {
                title: "Could not clean up".to_string(),
                text: err,
            }),
        }
        self.refresh_work(index);
    }

    fn on_setup_done(&mut self, index: usize, result: Result<SetupReport, String>) {
        self.setting_up.retain(|i| *i != index);
        match result {
            Ok(report) => {
                self.notify(
                    format!("{} is ready: {}", report.dir, report.access),
                    Tone::Info,
                );
                for note in report.notes {
                    self.notify(note, Tone::Warn);
                }
                self.refresh_status(index);
            }
            Err(err) => self.modals.push_back(Modal::Notice {
                title: "Guided setup failed".to_string(),
                text: err,
            }),
        }
    }

    pub fn on_question(&mut self, question: Question) {
        self.modals.push_back(match question {
            Question::Secret {
                title,
                prompt,
                reply,
            } => Modal::Secret {
                title,
                prompt,
                input: String::new(),
                reply: Some(reply),
            },
            Question::TrustHost {
                host,
                algorithm,
                fingerprint,
                reply,
            } => Modal::TrustHost {
                host,
                algorithm,
                fingerprint,
                reply: Some(reply),
            },
        });
    }
}
