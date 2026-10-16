//! What the app starts: connecting, controlling the service, opening logs and releases, deploys and rollbacks.

use std::collections::VecDeque;

use ratatui::widgets::TableState;
use tokio::sync::oneshot;

use super::{
    App, Fetch, Fetched, Live, LogsState, LogsView, Modal, OpKind, Operation, ReleasesView, Screen,
    Tone,
};
use shipdeck_engine::deploy;
use shipdeck_engine::deploy::report::{OpId, Reporter};
use shipdeck_engine::remote::{Control, Server, log_command};

impl App {
    /// Connects the deployable, or disconnects it when it is connected.
    pub(super) fn toggle_connection(&mut self, index: usize) {
        match self.live[index] {
            Live::Loading => self.notify("Still connecting", Tone::Info),
            Live::Ready(_) => self.disconnect(index),
            Live::Disconnected | Live::Error(_) => self.refresh_status(index),
        }
    }

    /// Closes the session to the deployable's server. Every deployable on
    /// the same `user@host:port` shares it, so they all show disconnected.
    pub(super) fn disconnect(&mut self, index: usize) {
        let endpoint = self.deployables[index].endpoint.clone();
        let in_use = self
            .op
            .as_ref()
            .filter(|op| op.running())
            .is_some_and(|op| self.deployables[op.deployable].endpoint == endpoint);
        if in_use {
            self.notify("An operation is using this connection", Tone::Warn);
            return;
        }
        for (deployable, live) in self.deployables.iter().zip(self.live.iter_mut()) {
            if deployable.endpoint == endpoint {
                *live = Live::Disconnected;
            }
        }
        self.notify(
            format!("Disconnected from {}", endpoint.label()),
            Tone::Info,
        );
        let pool = self.pool.clone();
        tokio::spawn(async move { pool.close(&endpoint).await });
    }

    /// Fetches the status again for every connected deployable.
    pub(super) fn refresh_connected(&mut self) {
        let connected: Vec<usize> = (0..self.live.len())
            .filter(|index| matches!(self.live[*index], Live::Ready(_)))
            .collect();
        if connected.is_empty() {
            self.notify("Nothing is connected. Press c to connect", Tone::Info);
            return;
        }
        for index in connected {
            self.refresh_status(index);
        }
        self.notify("Refreshing live status", Tone::Info);
    }

    pub fn refresh_status(&mut self, index: usize) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        self.live[index] = Live::Loading;
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .status()
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::Status { index, result });
        });
    }

    /// Stops, starts or restarts the deployable's service in the
    /// background.
    pub(super) fn control(&mut self, index: usize, action: Control) {
        if self.controlling.contains(&index) {
            self.notify("Wait for the last command to finish", Tone::Warn);
            return;
        }
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        self.controlling.push(index);
        let name = deployable.config.service.name.clone();
        self.notify(format!("{} {name}…", doing(action)), Tone::Info);
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .control(action)
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::Controlled {
                index,
                action,
                result,
            });
        });
    }

    pub(super) fn controlled(&mut self, index: usize, action: Control, result: Result<(), String>) {
        self.controlling.retain(|i| *i != index);
        let Some(deployable) = self.deployables.get(index) else {
            return;
        };
        let name = deployable.config.service.name.clone();
        match result {
            Ok(()) => {
                self.notify(format!("{name}: {}", done(action)), Tone::Info);
                self.refresh_status(index);
            }
            Err(text) => self.modals.push_back(Modal::Notice {
                title: format!("Could not {} {name}", action.verb()),
                text,
            }),
        }
    }

    /// Opens the log view and follows the deployable's log until it closes.
    pub(super) fn open_logs(&mut self, index: usize) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        let view = self.next_logs;
        self.next_logs += 1;
        let (stop, stopped) = oneshot::channel::<()>();
        self.logs = Some(LogsView {
            id: view,
            deployable: index,
            command: log_command(&deployable.config.service),
            lines: VecDeque::new(),
            scroll: 0,
            state: LogsState::Connecting,
            _stop: stop,
        });
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                let server = Server::new(session, &deployable.config.service);
                let _ = tx.send(Fetched::LogsOpen { view });
                let line_tx = tx.clone();
                server
                    .follow_logs(
                        |line| {
                            let _ = line_tx.send(Fetched::LogLine {
                                view,
                                line: line.to_string(),
                            });
                        },
                        async {
                            // The view closed: its sender is gone.
                            let _ = stopped.await;
                        },
                    )
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::LogsEnded { view, result });
        });
    }

    pub(super) fn open_releases(&mut self, index: usize) {
        let mut table = TableState::default();
        table.select(Some(0));
        self.releases = Some(ReleasesView {
            deployable: index,
            listing: Fetch::Loading,
            table,
        });
        self.screen = Screen::Releases;
        self.fetch_releases(index);
    }

    pub(super) fn fetch_releases(&mut self, index: usize) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        if let Some(view) = self.releases.as_mut().filter(|v| v.deployable == index) {
            view.listing = Fetch::Loading;
        }
        let (pool, tx) = (self.pool.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let result = async {
                let session = pool.get(&deployable.endpoint).await?;
                Server::new(session, &deployable.config.service)
                    .list()
                    .await
            }
            .await
            .map_err(|err| format!("{err:#}"));
            let _ = tx.send(Fetched::Releases { index, result });
        });
    }

    pub(super) fn start_deploy(&mut self, index: usize) {
        if self.op_running() {
            self.notify("An operation is already running", Tone::Warn);
            return;
        }
        let deployable = self.deployables[index].clone();
        let id = self.next_op_id();
        let steps = deploy::deploy_step_names(&deployable.config);
        self.op = Some(Operation::new(id, OpKind::Deploy, index, &steps));
        self.screen = Screen::Operation;
        let reporter = Reporter::new(id, self.ops.clone());
        tokio::spawn(deploy::deploy(deployable, self.pool.clone(), reporter));
    }

    pub(super) fn start_rollback(&mut self, index: usize, target: String) {
        if self.op_running() {
            self.notify("An operation is already running", Tone::Warn);
            return;
        }
        let deployable = self.deployables[index].clone();
        let id = self.next_op_id();
        let steps = deploy::rollback_step_names(&deployable.config);
        self.op = Some(Operation::new(
            id,
            OpKind::Rollback {
                target: target.clone(),
            },
            index,
            &steps,
        ));
        self.screen = Screen::Operation;
        let reporter = Reporter::new(id, self.ops.clone());
        tokio::spawn(deploy::rollback(
            deployable,
            target,
            self.pool.clone(),
            reporter,
        ));
    }

    pub(super) fn next_op_id(&mut self) -> OpId {
        let id = self.next_op;
        self.next_op += 1;
        id
    }
}

/// `Stopping`, `Starting`, `Restarting`.
pub(super) fn doing(action: Control) -> &'static str {
    match action {
        Control::Stop => "Stopping",
        Control::Start => "Starting",
        Control::Restart => "Restarting",
    }
}

/// `stopped`, `started`, `restarted`.
pub(super) fn done(action: Control) -> &'static str {
    match action {
        Control::Stop => "stopped",
        Control::Start => "started",
        Control::Restart => "restarted",
    }
}
