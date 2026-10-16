//! How results from the background change the app: fetches, operation updates and SSH questions.

use std::time::{Duration, Instant};

use super::{App, Asked, Fetch, Fetched, LOG_LIMIT, Live, LogsState, Modal, Screen};
use shipdeck_engine::deploy::report::{OpEvent, OpUpdate};
use shipdeck_engine::ssh::Question;

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
            Fetched::Controlled {
                index,
                action,
                result,
            } => self.controlled(index, action, result),
            Fetched::LogsOpen { view } => {
                if let Some(logs) = self.logs.as_mut().filter(|l| l.id == view) {
                    logs.state = LogsState::Following;
                }
            }
            Fetched::LogLine { view, line } => {
                if let Some(logs) = self.logs.as_mut().filter(|l| l.id == view) {
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
                if let Some(logs) = self.logs.as_mut().filter(|l| l.id == view) {
                    logs.state = LogsState::Ended(match result {
                        Ok(Some(code)) => format!("the log ended (exit {code})"),
                        Ok(None) => "the log ended".to_string(),
                        Err(err) => err,
                    });
                }
            }
        }
    }

    pub fn on_op(&mut self, update: OpUpdate) {
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
