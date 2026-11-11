//! One look at whether the service is up: systemd's or PM2's state, its
//! restart count and, if configured, the health URL.

use anyhow::Result;

use super::{Server, pairs};
use crate::config::Manager;
use crate::ssh::sh_quote;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpCheck {
    /// No health URL is configured.
    Skipped,
    Ok,
    Failed(String),
    /// The server has neither curl nor wget.
    NoClient,
}

#[derive(Debug, Clone)]
pub struct Probe {
    /// `systemctl is-active` output: `active`, `activating`, `failed`, ...
    /// PM2's state is put in the same words.
    pub state: String,
    /// The manager's restart counter; a rising count means a crash loop.
    pub restarts: Option<u64>,
    pub http: HttpCheck,
}

impl Probe {
    #[must_use]
    pub fn healthy(&self) -> bool {
        self.state == "active" && matches!(self.http, HttpCheck::Skipped | HttpCheck::Ok)
    }
}

impl Server {
    /// Fetches a health URL on the server; any 2xx answer is success.
    ///
    /// # Errors
    /// Returns an error when neither curl nor wget is there, or the URL fails.
    pub async fn fetch(&self, url: &str) -> Result<()> {
        let url = sh_quote(url);
        let script = format!(
            "if command -v curl >/dev/null 2>&1; then
  err=$(curl -fsS -o /dev/null --max-time 5 {url} 2>&1) && exit 0 || {{ echo \"$err\" >&2; exit 1; }}
elif command -v wget >/dev/null 2>&1; then
  wget -q -O /dev/null -T 5 {url}
else
  echo 'the server has neither curl nor wget' >&2; exit 127
fi"
        );
        self.session
            .exec(&script)
            .await?
            .check("fetching the health URL")?;
        Ok(())
    }

    /// Reads the service's state, restart count and health URL result.
    ///
    /// # Errors
    /// Returns an error when the server cannot be asked for its state.
    pub async fn probe(&self, url: Option<&str>) -> Result<Probe> {
        let svc = self.q_service();
        let http = match url {
            None => "echo http=skipped".to_string(),
            Some(url) => {
                let url = sh_quote(url);
                format!(
                    "if command -v curl >/dev/null 2>&1; then
  err=$(curl -fsS -o /dev/null --max-time 3 {url} 2>&1) && echo http=ok || echo \"http=fail:$(printf '%s' \"$err\" | head -n1)\"
elif command -v wget >/dev/null 2>&1; then
  wget -q -O /dev/null -T 3 {url} && echo http=ok || echo 'http=fail:wget could not fetch it'
else
  echo http=none
fi"
                )
            }
        };
        let (script, pm2) = match self.manager {
            None | Some(Manager::Oxmgr(_)) => (http, None),
            Some(Manager::Systemd(_)) => (
                format!(
                    "echo \"state=$(systemctl is-active {svc} 2>&1 | head -n1)\"
echo \"restarts=$(systemctl show -p NRestarts --value {svc} 2>/dev/null)\"
{http}"
                ),
                None,
            ),
            Some(Manager::Pm2(_)) => (http, Some(self.pm2_state().await?)),
        };
        let out = self.session.exec(&script).await?;
        let values = pairs(&out.stdout);
        let get = |key: &str| {
            values
                .iter()
                .find(|(k, _)| *k == key)
                .map_or("", |(_, v)| v.trim())
        };
        let http = match get("http") {
            "ok" => HttpCheck::Ok,
            "none" => HttpCheck::NoClient,
            "skipped" | "" => HttpCheck::Skipped,
            other => HttpCheck::Failed(other.strip_prefix("fail:").unwrap_or(other).to_string()),
        };
        let (state, restarts) = match (self.manager.as_ref(), pm2) {
            (None | Some(Manager::Oxmgr(_)), _) => ("active".to_string(), None),
            (_, Some(found)) => found,
            (_, None) => (get("state").to_string(), get("restarts").parse().ok()),
        };
        Ok(Probe {
            state,
            restarts,
            http,
        })
    }
}
