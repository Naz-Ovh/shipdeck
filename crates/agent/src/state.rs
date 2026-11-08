//! Numbered SQLite events for reconnect and inspection.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use shipdeck_protocol::message::agent::{
    AgentEvent, AgentEventAppend, AgentEventSaved, AgentEvents, AgentEventsAfter, AgentEventsRecent,
};

struct Store(Connection);

impl Store {
    /// Opens the database, creating its first schema under a write lock.
    fn open(path: &Path) -> Result<Self> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
                    bail!("the state database cannot be a symbolic link");
                }
            }
            Err(err) => return Err(err).context("creating the state database"),
        }
        let mut db = Connection::open(path).context("opening the state database")?;
        db.busy_timeout(Duration::from_secs(5))?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 1 {
            bail!("the state database belongs to a newer agent");
        }
        if version == 0 {
            tx.execute_batch("CREATE TABLE events (sequence INTEGER PRIMARY KEY AUTOINCREMENT, project TEXT NOT NULL, operation TEXT NOT NULL, kind TEXT NOT NULL, detail TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); PRAGMA user_version = 1;")?;
        }
        tx.commit()?;
        Ok(Self(db))
    }

    fn append(&self, event: &AgentEventAppend) -> Result<AgentEventSaved> {
        if event.project.is_empty()
            || event.project.len() > 4096
            || event.operation.is_empty()
            || event.operation.len() > 256
            || event.kind.is_empty()
            || event.kind.len() > 128
            || event.detail.len() > 65536
        {
            bail!("invalid event fields");
        }
        self.0.execute(
            "INSERT INTO events (project, operation, kind, detail) VALUES (?1, ?2, ?3, ?4)",
            params![event.project, event.operation, event.kind, event.detail],
        )?;
        Ok(AgentEventSaved {
            sequence: self.0.last_insert_rowid(),
        })
    }

    fn after(&self, request: &AgentEventsAfter) -> Result<AgentEvents> {
        if request.sequence < 0 || !(1..=1000).contains(&request.limit) {
            bail!("invalid event cursor or page size");
        }
        check_project(request.project.as_deref())?;
        let mut query = self.0.prepare(
            "SELECT sequence, project, operation, kind, detail, created_at FROM events \
             WHERE sequence > ?1 AND (?2 IS NULL OR project = ?2) ORDER BY sequence LIMIT ?3",
        )?;
        let rows = query.query_map(
            params![request.sequence, request.project, request.limit],
            event_row,
        )?;
        Ok(AgentEvents {
            events: rows.collect::<rusqlite::Result<Vec<_>>>()?,
        })
    }

    fn recent(&self, request: &AgentEventsRecent) -> Result<AgentEvents> {
        check_project(Some(&request.project))?;
        if !(1..=1000).contains(&request.limit) {
            bail!("invalid event page size");
        }
        let mut query = self.0.prepare(
            "SELECT sequence, project, operation, kind, detail, created_at FROM events \
             WHERE project = ?1 ORDER BY sequence DESC LIMIT ?2",
        )?;
        let rows = query.query_map(params![request.project, request.limit], event_row)?;
        let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        // The newest page, in the order they happened.
        events.reverse();
        Ok(AgentEvents { events })
    }

    /// The last activation event of one release that was not a health pass.
    fn unconfirmed_activation(&self, project: &str, operation: &str) -> Result<Option<String>> {
        let mut query = self.0.prepare(
            "SELECT kind FROM events WHERE project = ?1 AND operation = ?2 \
             AND kind IN ('switch_started', 'switch_completed', 'service_restarted', \
                          'health_passed') ORDER BY sequence DESC LIMIT 1",
        )?;
        let mut rows = query.query(params![project, operation])?;
        Ok(match rows.next()? {
            Some(row) => {
                let kind: String = row.get(0)?;
                (kind != "health_passed").then_some(kind)
            }
            None => None,
        })
    }
}

fn check_project(project: Option<&str>) -> Result<()> {
    if project.is_some_and(|p| p.is_empty() || p.len() > 4096) {
        bail!("invalid event project");
    }
    Ok(())
}

fn event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentEvent> {
    Ok(AgentEvent {
        sequence: row.get(0)?,
        project: row.get(1)?,
        operation: row.get(2)?,
        kind: row.get(3)?,
        detail: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// Runs database work outside the asynchronous SSH reader.
pub(crate) async fn append(event: AgentEventAppend) -> Result<AgentEventSaved> {
    tokio::task::spawn_blocking(move || Store::open(&state_path()?)?.append(&event)).await?
}

/// Reads the next page of events outside the asynchronous SSH reader.
pub(crate) async fn after(request: AgentEventsAfter) -> Result<AgentEvents> {
    tokio::task::spawn_blocking(move || Store::open(&state_path()?)?.after(&request)).await?
}

/// Reads the newest page of one project's events.
pub(crate) async fn recent(request: AgentEventsRecent) -> Result<AgentEvents> {
    tokio::task::spawn_blocking(move || Store::open(&state_path()?)?.recent(&request)).await?
}

/// The last activation event when it never reached a health pass.
pub(crate) async fn unconfirmed_activation(
    project: &str,
    operation: &str,
) -> Result<Option<String>> {
    let Some(path) = existing_state_path() else {
        return Ok(None);
    };
    let (project, operation) = (project.to_string(), operation.to_string());
    tokio::task::spawn_blocking(move || {
        Store::open(&path)?.unconfirmed_activation(&project, &operation)
    })
    .await?
}

/// The state file when it already exists; nothing is created.
fn existing_state_path() -> Option<PathBuf> {
    let path = home().ok()?.join(".shipdeck").join("state.db");
    path.is_file().then_some(path)
}

/// The deploy user's home, passed at launch, so a root agent does not move
/// the event history to `/root`.
fn home() -> Result<PathBuf> {
    let home = std::env::var_os("SHIPDECK_OPERATOR_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .context("the agent did not receive its operator home")?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        bail!("the agent operator home must be absolute");
    }
    Ok(home)
}

fn state_path() -> Result<PathBuf> {
    let folder = home()?.join(".shipdeck");
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&folder) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err).context("creating the state directory"),
    }
    let metadata = std::fs::symlink_metadata(&folder)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("the state directory must be a directory, not a symbolic link");
    }
    Ok(folder.join("state.db"))
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
