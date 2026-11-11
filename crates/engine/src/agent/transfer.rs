//! Sends a release over a separate SSH channel to the connected agent.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use shipdeck_protocol::message::agent::{UPLOAD_PREPARE, UploadPrepare, UploadReservation};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::Agent;
use super::layout;
use crate::ssh::Session;

impl Agent {
    /// Streams one verified release archive on a separate SSH channel.
    ///
    /// # Errors
    /// Returns an error if reservation, streaming, or remote verification fails.
    pub async fn upload_release(
        &self,
        session: &Session,
        root: &str,
        release: &str,
        data: &[u8],
        retry: bool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        let total = u64::try_from(data.len())?;
        let lock_token = self
            .lock_token()
            .context("the project lock token is missing for this upload")?;
        let request = UploadPrepare {
            root: root.to_string(),
            release: release.to_string(),
            bytes: total,
            sha256: base16ct::lower::encode_string(&Sha256::digest(data)),
            lock_token,
            retry,
        };
        let reservation: UploadReservation = self
            .handle
            .request(UPLOAD_PREPARE, &request)
            .await
            .context("reserving the agent upload")?;
        let command = layout::receiver_command(
            &self.prefix,
            &self.path,
            &self.operator,
            &reservation.reservation,
        );
        let mut stream = session.open_stream(&command).await?;
        let mut sent = 0_u64;
        for chunk in data.chunks(256 * 1024) {
            // A receiver that died leaves the channel half-open; without a
            // limit the write waits forever and the app cannot report it.
            tokio::time::timeout(Duration::from_secs(60), stream.write_all(chunk))
                .await
                .context("the upload stopped responding; unfinished files were kept")?
                .context("sending the release to the agent")?;
            sent = sent
                .checked_add(u64::try_from(chunk.len())?)
                .context("upload length overflow")?;
            progress(sent, total);
        }
        tokio::time::timeout(Duration::from_secs(30), stream.shutdown())
            .await
            .context("the upload did not end; unfinished files were kept")?
            .context("ending the upload")?;
        let mut answer = [0_u8; 3];
        tokio::time::timeout(Duration::from_secs(30), stream.read_exact(&mut answer))
            .await
            .context("the agent did not confirm the upload")?
            .context("the agent did not accept the upload; see agent.log on the server")?;
        if answer != *b"ok\n" {
            bail!("the agent sent an invalid upload confirmation");
        }
        Ok(())
    }
}
