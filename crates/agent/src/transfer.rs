//! One-use reservations for binary uploads on a second SSH channel.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shipdeck_protocol::message::agent::{AgentEventAppend, UploadPrepare, UploadReservation};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

const RESERVATION_LIFETIME: Duration = Duration::from_mins(15);

#[derive(Serialize, Deserialize)]
struct Reservation {
    request: UploadPrepare,
    releases: PathBuf,
    expires: u64,
}

/// Reserves a release archive and creates a private, one-use receiver ticket.
pub(crate) async fn prepare(request: &UploadPrepare) -> Result<UploadReservation> {
    if !Path::new(&request.root).is_absolute() || !safe_id(&request.release) {
        bail!("invalid deployment root or release name");
    }
    if request.lock_token == 0 {
        bail!("invalid project lock token");
    }
    if request.sha256.len() != 64
        || !request
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid SHA-256 checksum");
    }
    let releases = tokio::fs::canonicalize(Path::new(&request.root).join("releases"))
        .await
        .context("opening the releases directory")?;
    match tokio::fs::symlink_metadata(releases.join(&request.release)).await {
        Ok(_) => bail!("the release already exists"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("checking for an existing release"),
    }
    if !request.retry {
        let prefix = format!(".{}.", request.release);
        let mut entries = tokio::fs::read_dir(&releases).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
            {
                bail!(
                    "unfinished files already exist for this release; inspect them before cleanup"
                );
            }
        }
    }
    let destination = releases.join(format!(".{}.tar.zst", request.release));
    if tokio::fs::try_exists(&destination).await? {
        bail!("the release archive already exists");
    }
    let folder = private_folder().await?;
    let expires = now()?
        .checked_add(RESERVATION_LIFETIME.as_secs())
        .context("reservation expiry overflow")?;
    let reservation = Reservation {
        request: request.clone(),
        releases,
        expires,
    };
    let ticket = folder.join("pending");
    tokio::fs::write(&ticket, serde_json::to_vec(&reservation)?).await?;
    Ok(UploadReservation {
        reservation: ticket.to_string_lossy().into_owned(),
    })
}

/// Reserves the upload and records its start in the same request.
pub(crate) async fn prepare_recorded(request: UploadPrepare) -> Result<UploadReservation> {
    let reservation = prepare(&request).await?;
    record(&request, "upload_started").await?;
    Ok(reservation)
}

/// Receives a reserved upload and records its verification before confirming it.
///
/// # Errors
/// Returns an error when receipt, verification, or event storage fails.
pub async fn receive_recorded<R: AsyncRead + Unpin>(ticket: &str, input: R) -> Result<()> {
    let request = receive(ticket, input).await?;
    record(&request, "upload_verified").await
}

async fn record(request: &UploadPrepare, kind: &str) -> Result<()> {
    crate::state::append(AgentEventAppend {
        project: request.root.clone(),
        operation: request.release.clone(),
        kind: kind.to_string(),
        detail: format!("{} bytes", request.bytes),
    })
    .await
    .context("saving the upload event")?;
    Ok(())
}

/// Receives and checks the reserved bytes; partial data remains on failure.
///
/// # Errors
/// Returns an error if the ticket is invalid, the stream is incomplete, or verification fails.
async fn receive<R: AsyncRead + Unpin>(ticket: &str, mut input: R) -> Result<UploadPrepare> {
    let ticket = Path::new(ticket);
    if ticket.file_name().is_none_or(|name| name != "pending") {
        bail!("invalid upload reservation");
    }
    let folder = ticket.parent().context("missing reservation folder")?;
    let claimed = folder.join("claimed");
    tokio::fs::rename(ticket, &claimed)
        .await
        .context("claiming upload reservation")?;
    let reservation: Reservation = serde_json::from_slice(&tokio::fs::read(&claimed).await?)?;
    if now()? > reservation.expires {
        bail!("upload reservation expired");
    }
    let root = reservation
        .releases
        .parent()
        .context("the reservation has no project folder")?;
    verify_lock(root, reservation.request.lock_token)?;
    let destination = reservation
        .releases
        .join(format!(".{}.tar.zst", reservation.request.release));
    let token = folder
        .file_name()
        .context("missing upload token")?
        .to_string_lossy();
    let partial = reservation.releases.join(format!(
        ".{}.{}.partial",
        reservation.request.release, token
    ));
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&partial)
        .await
        .context("creating upload file")?;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut remaining = reservation.request.bytes;
    let mut hash = Sha256::new();
    while remaining > 0 {
        let limit = usize::try_from(remaining.min(u64::try_from(buffer.len())?))?;
        let count = input
            .read(&mut buffer[..limit])
            .await
            .context("reading upload")?;
        if count == 0 {
            bail!("upload ended before the declared length");
        }
        file.write_all(&buffer[..count])
            .await
            .context("writing upload")?;
        hash.update(&buffer[..count]);
        remaining -= u64::try_from(count)?;
        // A ticket must stop working when its lock goes away.
        if remaining % (4 * 1024 * 1024) < u64::try_from(count)? {
            verify_lock(root, reservation.request.lock_token)?;
        }
    }
    if input.read(&mut buffer[..1]).await? != 0 {
        bail!("upload exceeded the declared length");
    }
    if base16ct::lower::encode_string(&hash.finalize()) != reservation.request.sha256 {
        bail!("upload checksum did not match");
    }
    file.sync_all().await.context("saving upload")?;
    drop(file);
    tokio::fs::hard_link(&partial, &destination)
        .await
        .context("finishing upload")?;
    tokio::fs::remove_file(&partial)
        .await
        .context("removing completed upload link")?;
    Ok(reservation.request)
}

/// Checks that the reservation's project lock is still held by its token.
fn verify_lock(root: &Path, token: u64) -> Result<()> {
    let path = root.join(".shipdeck.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .context("opening the project lock for the upload")?;
    match file.try_lock() {
        Ok(()) => {
            drop(file);
            bail!("the project lock was released; the upload ticket is no longer valid");
        }
        Err(std::fs::TryLockError::WouldBlock) => {}
        Err(std::fs::TryLockError::Error(err)) => {
            return Err(err).context("checking the project lock for the upload");
        }
    }
    let text = std::fs::read_to_string(&path).context("reading the project lock token")?;
    if text.trim() != token.to_string() {
        bail!(
            "the project lock belongs to another operation; the upload ticket is no longer valid"
        );
    }
    Ok(())
}

pub(crate) fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._+-".contains(ch))
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

/// Uses an unpredictable private directory as the receiver's ticket.
async fn private_folder() -> Result<PathBuf> {
    let token = random_token().await?;
    let folder = std::env::temp_dir().join(format!(".shipdeck-upload-{token}"));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&folder)
        .context("creating private upload reservation")?;
    Ok(folder)
}

/// Makes an unpredictable name for private staging artifacts.
pub(crate) async fn random_token() -> Result<String> {
    let mut random = [0_u8; 24];
    tokio::fs::File::open("/dev/urandom")
        .await?
        .read_exact(&mut random)
        .await?;
    Ok(base16ct::lower::encode_string(&random))
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
