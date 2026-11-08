//! Typed agent operations the engine uses beside the older SSH scripts.

use anyhow::{Context, Result};

use shipdeck_protocol::message::agent::{
    AgentBackupCreate, AgentBackupCreated, AgentBackupRestore, AgentCleaned, AgentCleanupWork,
    AgentDiscard, AgentDropRequest, AgentEventAppend, AgentEventSaved, AgentEvents,
    AgentEventsAfter, AgentEventsRecent, AgentLinkShared, AgentLogsFollow, AgentLogsStop,
    AgentLogsView, AgentOk, AgentReleaseContents, AgentReleaseContentsRequest, AgentReleaseFile,
    AgentReleaseFilesRequest, AgentReleaseRead, AgentReleaseReadRequest, AgentReleases,
    AgentReleasesRequest, AgentService, AgentServiceAction, AgentServiceControl, AgentStatus,
    BACKUP_CREATE, BACKUP_RESTORE, CLEANUP_WORK, DISCARD_RELEASE, EVENT_APPEND, EVENTS_AFTER,
    EVENTS_RECENT, LINK_SHARED, LOGS_FOLLOW, LOGS_STOP, RELEASE_CONTENTS, RELEASE_DROP,
    RELEASE_FILES, RELEASE_READ_FILE, RELEASES_LIST, SERVICE_CONTROL, SERVICE_STATUS,
};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::Agent;

/// Something the agent reports about one followed log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLogEvent {
    Line(String),
    Ended {
        code: Option<u32>,
        error: Option<String>,
    },
}

impl Agent {
    /// Reads the service's live state through the agent.
    ///
    /// # Errors
    /// Returns an error if the agent cannot run the service tools.
    pub async fn service_status(&self, service: &AgentService) -> Result<AgentStatus> {
        self.handle
            .request(SERVICE_STATUS, service)
            .await
            .context("reading the service state")
    }

    /// Stops, starts or restarts the service through the agent.
    ///
    /// # Errors
    /// Returns an error if the service tools refuse the change.
    pub async fn service_control(
        &self,
        service: &AgentService,
        action: AgentServiceAction,
    ) -> Result<()> {
        let _: AgentOk = self
            .handle
            .request(
                SERVICE_CONTROL,
                &AgentServiceControl {
                    service: service.clone(),
                    action,
                },
            )
            .await
            .context("changing the service")?;
        Ok(())
    }

    /// Lists the releases and backups in one deployment folder.
    ///
    /// # Errors
    /// Returns an error if the folder or its entries cannot be read.
    pub async fn list_releases(&self, root: &str) -> Result<AgentReleases> {
        self.handle
            .request(
                RELEASES_LIST,
                &AgentReleasesRequest {
                    root: root.to_string(),
                },
            )
            .await
            .context("listing releases")
    }

    /// Lists every path of one release, without following links.
    ///
    /// # Errors
    /// Returns an error if the release cannot be read.
    pub async fn release_files(&self, root: &str, release: &str) -> Result<Vec<AgentReleaseFile>> {
        let answer: shipdeck_protocol::message::agent::AgentReleaseFiles = self
            .handle
            .request(
                RELEASE_FILES,
                &AgentReleaseFilesRequest {
                    root: root.to_string(),
                    release: release.to_string(),
                },
            )
            .await
            .context("listing release files")?;
        Ok(answer.files)
    }

    /// Lists a release folder or its backup through the agent.
    ///
    /// # Errors
    /// Returns an error if the release cannot be read.
    pub async fn release_contents(
        &self,
        root: &str,
        release: &str,
        backup: bool,
    ) -> Result<AgentReleaseContents> {
        self.handle
            .request(
                RELEASE_CONTENTS,
                &AgentReleaseContentsRequest {
                    root: root.to_string(),
                    release: release.to_string(),
                    backup,
                },
            )
            .await
            .context("reading release contents")
    }

    /// Reads the start of one release file through the agent.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read.
    pub async fn read_release_file(
        &self,
        root: &str,
        release: &str,
        backup: bool,
        path: &str,
        cap: u64,
    ) -> Result<AgentReleaseRead> {
        self.handle
            .request(
                RELEASE_READ_FILE,
                &AgentReleaseReadRequest {
                    root: root.to_string(),
                    release: release.to_string(),
                    backup,
                    path: path.to_string(),
                    cap,
                },
            )
            .await
            .context("reading a release file")
    }

    /// Removes one opened release or one backup through the agent.
    ///
    /// # Errors
    /// Returns an error if the release or backup cannot be removed.
    pub async fn drop_release(
        &self,
        root: &str,
        release: &str,
        open: bool,
        backup: bool,
    ) -> Result<()> {
        let _: AgentOk = self
            .handle
            .request(
                RELEASE_DROP,
                &AgentDropRequest {
                    root: root.to_string(),
                    release: release.to_string(),
                    open,
                    backup,
                },
            )
            .await
            .context("removing a kept release")?;
        Ok(())
    }

    /// Links each shared folder into a prepared release.
    ///
    /// # Errors
    /// Returns an error if a link already exists or cannot be made.
    pub async fn link_shared(&self, root: &str, release: &str, shared: &[String]) -> Result<()> {
        let _: AgentOk = self
            .handle
            .request(
                LINK_SHARED,
                &AgentLinkShared {
                    root: root.to_string(),
                    release: release.to_string(),
                    shared: shared.to_vec(),
                },
            )
            .await
            .context("linking shared folders")?;
        Ok(())
    }

    /// Packs the chosen release paths into its backup.
    ///
    /// # Errors
    /// Returns an error if the backup cannot be written.
    pub async fn create_backup(&self, root: &str, release: &str, paths: &[String]) -> Result<u64> {
        let answer: AgentBackupCreated = self
            .handle
            .request(
                BACKUP_CREATE,
                &AgentBackupCreate {
                    root: root.to_string(),
                    release: release.to_string(),
                    paths: paths.to_vec(),
                },
            )
            .await
            .context("packing the backup")?;
        Ok(answer.bytes)
    }

    /// Unpacks a release's backup beside it.
    ///
    /// # Errors
    /// Returns an error if the backup cannot be unpacked.
    pub async fn restore_backup(&self, root: &str, release: &str) -> Result<()> {
        let _: AgentOk = self
            .handle
            .request(
                BACKUP_RESTORE,
                &AgentBackupRestore {
                    root: root.to_string(),
                    release: release.to_string(),
                },
            )
            .await
            .context("restoring the backup")?;
        Ok(())
    }

    /// Removes the artifacts the owner approved.
    ///
    /// # Errors
    /// Returns an error if any path is not an approved artifact.
    pub async fn cleanup_work(&self, root: &str, artifacts: &[String]) -> Result<Vec<String>> {
        let answer: AgentCleaned = self
            .handle
            .request(
                CLEANUP_WORK,
                &AgentCleanupWork {
                    root: root.to_string(),
                    artifacts: artifacts.to_vec(),
                },
            )
            .await
            .context("cleaning unfinished files")?;
        Ok(answer.removed)
    }

    /// Removes one failed release and what was sent to make it.
    ///
    /// # Errors
    /// Returns an error if the release name is unsafe or removal fails.
    pub async fn discard_release(&self, root: &str, release: &str) -> Result<()> {
        let _: AgentOk = self
            .handle
            .request(
                DISCARD_RELEASE,
                &AgentDiscard {
                    root: root.to_string(),
                    release: release.to_string(),
                },
            )
            .await
            .context("removing the failed release")?;
        Ok(())
    }

    /// Saves one numbered event for a project.
    ///
    /// # Errors
    /// Returns an error if the event database refuses it.
    pub async fn record_event(
        &self,
        project: &str,
        operation: &str,
        kind: &str,
        detail: &str,
    ) -> Result<i64> {
        let saved: AgentEventSaved = self
            .handle
            .request(
                EVENT_APPEND,
                &AgentEventAppend {
                    project: project.to_string(),
                    operation: operation.to_string(),
                    kind: kind.to_string(),
                    detail: detail.to_string(),
                },
            )
            .await
            .context("saving an event")?;
        Ok(saved.sequence)
    }

    /// Reads a page of events after a sequence, for one project or all.
    ///
    /// # Errors
    /// Returns an error if the event database cannot be read.
    pub async fn events_after(
        &self,
        project: Option<&str>,
        sequence: i64,
        limit: u32,
    ) -> Result<AgentEvents> {
        self.handle
            .request(
                EVENTS_AFTER,
                &AgentEventsAfter {
                    project: project.map(str::to_string),
                    sequence,
                    limit,
                },
            )
            .await
            .context("reading agent events")
    }

    /// The newest events of one project, oldest first.
    ///
    /// # Errors
    /// Returns an error if the event database cannot be read.
    pub async fn recent_events(&self, project: &str, limit: u32) -> Result<AgentEvents> {
        self.handle
            .request(
                EVENTS_RECENT,
                &AgentEventsRecent {
                    project: project.to_string(),
                    limit,
                },
            )
            .await
            .context("reading recent events")
    }

    /// Starts following a service's log through the agent.
    ///
    /// # Errors
    /// Returns an error if the agent cannot start the log command.
    pub async fn open_log(
        &self,
        service: &AgentService,
        lines: u32,
    ) -> Result<(u64, UnboundedReceiver<AgentLogEvent>)> {
        let view: AgentLogsView = self
            .handle
            .request(
                LOGS_FOLLOW,
                &AgentLogsFollow {
                    service: service.clone(),
                    lines,
                },
            )
            .await
            .context("starting the service log")?;
        let (sender, receiver) = mpsc::unbounded_channel();
        self.logs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(view.view, sender);
        Ok((view.view, receiver))
    }

    /// Stops one followed log.
    pub async fn stop_log(&self, view: u64) {
        let _ = self
            .handle
            .request::<_, AgentOk>(LOGS_STOP, &AgentLogsStop { view })
            .await;
    }
}

/// Delivers one log notification to the receiver that follows it.
pub(super) fn deliver_log(
    logs: &super::session::LogViews,
    method: &str,
    params: serde_json::Value,
) {
    match method {
        shipdeck_protocol::message::agent::LOGS_LINE => {
            if let Ok(line) =
                serde_json::from_value::<shipdeck_protocol::message::agent::AgentLogLine>(params)
            {
                let logs = logs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(sender) = logs.get(&line.view) {
                    let _ = sender.send(AgentLogEvent::Line(line.line));
                }
            }
        }
        shipdeck_protocol::message::agent::LOGS_ENDED => {
            if let Ok(ended) =
                serde_json::from_value::<shipdeck_protocol::message::agent::AgentLogsEnded>(params)
            {
                let sender = logs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&ended.view);
                if let Some(sender) = sender {
                    let _ = sender.send(AgentLogEvent::Ended {
                        code: ended.code,
                        error: ended.error,
                    });
                }
            }
        }
        _ => {}
    }
}
