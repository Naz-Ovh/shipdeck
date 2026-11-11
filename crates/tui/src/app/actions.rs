//! What the app starts: connecting, controlling the service, opening logs and
//! releases, deploys and rollbacks. Each one is a request to the engine.

use std::collections::VecDeque;

use ratatui::widgets::TableState;

use shipdeck_protocol::message::agent::ServerFacts;
use shipdeck_protocol::message::method;
use shipdeck_protocol::message::request::{
    ConnectionClose, DeployStart, Done, LogsFollow, LogsFollowResult, LogsStop, OperationStarted,
    ReleasesList, ReleasesListResult, RollbackStart, ServerSetup, ServiceControl, SetupAccess,
    SetupReport, StatusGet, StatusGetResult, WorkCleanup, WorkCleanupResult, WorkGet, WorkReport,
};

use shipdeck_engine::remote::{Control, log_command};

use super::{
    App, Fetch, Fetched, Live, LogsState, LogsView, Modal, OpKind, ReleasesView, Screen, Tone,
};
use crate::convert;

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
        let asked = ConnectionClose {
            deployable: self.deployables[index].slug.clone(),
        };
        let engine = self.engine.clone();
        tokio::spawn(async move {
            let _ = engine
                .request::<_, Done>(method::CONNECTION_CLOSE, &asked)
                .await;
        });
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
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let asked = StatusGet {
                deployable: deployable.slug.clone(),
            };
            let result = engine
                .request::<_, StatusGetResult>(method::STATUS_GET, &asked)
                .await
                .map(convert::status)
                .map_err(|err| err.reason());
            let _ = tx.send(Fetched::Status { index, result });
        });
        self.refresh_facts(index);
        self.refresh_work(index);
    }

    /// Reads the server's Linux, CPU, memory, disk and tools.
    pub(super) fn refresh_facts(&mut self, index: usize) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let asked = shipdeck_protocol::message::request::ServerInfoGet {
                deployable: deployable.slug.clone(),
            };
            let result = engine
                .request::<_, ServerFacts>(method::SERVER_INFO, &asked)
                .await
                .map_err(|err| err.reason());
            let _ = tx.send(Fetched::Facts { index, result });
        });
    }

    /// Reads the live release, unfinished files, and recent events.
    pub(super) fn refresh_work(&mut self, index: usize) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let asked = WorkGet {
                deployable: deployable.slug.clone(),
            };
            let result = engine
                .request::<_, WorkReport>(method::WORK_GET, &asked)
                .await
                .map_err(|err| err.reason());
            let _ = tx.send(Fetched::Work { index, result });
        });
    }

    /// Removes the reported unfinished files the user approved.
    pub(super) fn cleanup_work(&mut self, index: usize, artifacts: Vec<String>) {
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let asked = WorkCleanup {
                deployable: deployable.slug.clone(),
                artifacts,
            };
            let result = engine
                .request::<_, WorkCleanupResult>(method::WORK_CLEANUP, &asked)
                .await
                .map(|result| result.removed)
                .map_err(|err| err.reason());
            let _ = tx.send(Fetched::Cleaned { index, result });
        });
    }

    /// Runs guided setup with the chosen access and the sudo password.
    pub(super) fn start_setup(
        &mut self,
        index: usize,
        access: SetupAccess,
        password: Option<String>,
    ) {
        if self.setting_up.contains(&index) {
            self.notify("Guided setup is already running", Tone::Warn);
            return;
        }
        let Some(deployable) = self.deployables.get(index).cloned() else {
            return;
        };
        self.setting_up.push(index);
        self.notify("Preparing the server…", Tone::Info);
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let asked = ServerSetup {
                deployable: deployable.slug.clone(),
                access,
                sudo_password: password,
            };
            let result = engine
                .request::<_, SetupReport>(method::SERVER_SETUP, &asked)
                .await
                .map_err(|err| err.reason());
            let _ = tx.send(Fetched::SetupDone { index, result });
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
        if deployable.first_process().is_none() {
            self.notify(
                "Static site: there is no process to start, stop or restart",
                Tone::Warn,
            );
            return;
        }
        self.controlling.push(index);
        let name = deployable.first_process().map_or_else(
            || deployable.name().to_string(),
            |app| deployable.process_name(app),
        );
        self.notify(format!("{} {name}…", doing(action)), Tone::Info);
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let asked = ServiceControl {
                deployable: deployable.slug.clone(),
                action: convert::action(action),
            };
            let result = engine
                .request::<_, Done>(method::SERVICE_CONTROL, &asked)
                .await
                .map(|_| ())
                .map_err(|err| err.reason());
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
        let name = deployable.first_process().map_or_else(
            || deployable.name().to_string(),
            |app| deployable.process_name(app),
        );
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
        if deployable.first_process().is_none() {
            self.notify(
                "Static site: no process logs; nginx keeps its own in /var/log/nginx",
                Tone::Warn,
            );
            return;
        }
        let manager = deployable
            .first_process()
            .and_then(|app| deployable.manager(app));
        let process = deployable
            .first_process()
            .map_or_else(String::new, |app| deployable.process_name(app));
        self.logs = Some(LogsView {
            id: None,
            deployable: index,
            command: log_command(manager, &process),
            lines: VecDeque::new(),
            scroll: 0,
            state: LogsState::Connecting,
        });
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let asked = LogsFollow {
                deployable: deployable.slug.clone(),
            };
            match engine
                .request::<_, LogsFollowResult>(method::LOGS_FOLLOW, &asked)
                .await
            {
                Ok(follow) => {
                    let _ = tx.send(Fetched::LogsOpen { view: follow.view });
                }
                Err(err) => {
                    let _ = tx.send(Fetched::LogsFailed {
                        result: err.reason(),
                    });
                }
            }
        });
    }

    /// Stops a live log the view is done with.
    pub(super) fn stop_log(&self, view: u64) {
        let asked = LogsStop { view };
        let engine = self.engine.clone();
        tokio::spawn(async move {
            let _ = engine.request::<_, Done>(method::LOGS_STOP, &asked).await;
        });
    }

    pub(super) fn open_releases(&mut self, index: usize) {
        let mut table = TableState::default();
        table.select(Some(0));
        self.releases = Some(ReleasesView {
            deployable: index,
            listing: Fetch::Loading,
            table,
            details: None,
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
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        tokio::spawn(async move {
            let asked = ReleasesList {
                deployable: deployable.slug.clone(),
            };
            let result = engine
                .request::<_, ReleasesListResult>(method::RELEASES_LIST, &asked)
                .await
                .map(convert::listing)
                .map_err(|err| err.reason());
            let _ = tx.send(Fetched::Releases { index, result });
        });
    }

    pub(super) fn start_deploy(&mut self, index: usize) {
        if self.busy() {
            self.notify("An operation is already running", Tone::Warn);
            return;
        }
        let deployable = self.deployables[index].clone();
        self.starting = true;
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        let kind = OpKind::Deploy;
        tokio::spawn(async move {
            let asked = DeployStart {
                deployable: deployable.slug.clone(),
                on_failure: None,
            };
            let result = engine
                .request::<_, OperationStarted>(method::DEPLOY_START, &asked)
                .await
                .map(|started| (started.op, started.steps))
                .map_err(|err| err.reason());
            let _ = tx.send(Fetched::Started {
                index,
                kind,
                result,
            });
        });
    }

    pub(super) fn start_rollback(&mut self, index: usize, target: String) {
        if self.busy() {
            self.notify("An operation is already running", Tone::Warn);
            return;
        }
        let deployable = self.deployables[index].clone();
        self.starting = true;
        let (engine, tx) = (self.engine.clone(), self.fetched.clone());
        let kind = OpKind::Rollback {
            target: target.clone(),
        };
        tokio::spawn(async move {
            let asked = RollbackStart {
                deployable: deployable.slug.clone(),
                release: target,
            };
            let result = engine
                .request::<_, OperationStarted>(method::ROLLBACK_START, &asked)
                .await
                .map(|started| (started.op, started.steps))
                .map_err(|err| err.reason());
            let _ = tx.send(Fetched::Started {
                index,
                kind,
                result,
            });
        });
    }

    /// Whether a deploy or rollback is running, or on its way.
    fn busy(&self) -> bool {
        self.op_running() || self.starting
    }

    /// Says goodbye, so the engine stops taking work and exits.
    pub async fn close(&self) {
        let _ = self
            .engine
            .request::<_, Done>(method::SHUTDOWN, &Done {})
            .await;
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
