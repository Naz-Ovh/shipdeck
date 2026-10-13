//! Checking the server's host key against `~/.ssh/known_hosts`, and asking
//! before trusting a server seen for the first time.

use std::sync::{Arc, Mutex};

use russh::client;
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::Question;

pub struct Client {
    host: String,
    port: u16,
    asker: UnboundedSender<Question>,
    /// Why the host key was refused, for a clearer error than russh's own.
    rejected: Arc<Mutex<Option<String>>>,
}

impl Client {
    pub fn new(host: String, port: u16, asker: UnboundedSender<Question>) -> Self {
        Self {
            host,
            port,
            asker,
            rejected: Arc::default(),
        }
    }

    pub fn rejection(&self) -> Arc<Mutex<Option<String>>> {
        self.rejected.clone()
    }

    fn reject(&self, reason: String) -> bool {
        *self.rejected.lock().expect("rejection lock") = Some(reason);
        false
    }

    fn host_label(&self) -> String {
        if self.port == 22 {
            self.host.clone()
        } else {
            format!("[{}]:{}", self.host, self.port)
        }
    }
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_key else {
            return Ok(self.reject(format!(
                "{} offered a host certificate; only plain host keys are supported",
                self.host
            )));
        };

        match russh::keys::check_known_hosts(&self.host, self.port, key) {
            Ok(true) => return Ok(true),
            Err(russh::keys::Error::KeyChanged { line }) => {
                return Ok(self.reject(format!(
                    "HOST KEY CHANGED for {}: it differs from line {line} of ~/.ssh/known_hosts. \
                     Someone could be intercepting the connection. If the server was reinstalled, \
                     remove that line and connect again.",
                    self.host_label()
                )));
            }
            // Not recorded yet, or no known_hosts file at all.
            Ok(false) | Err(_) => {}
        }

        let (reply, answer) = oneshot::channel();
        let asked = self.asker.send(Question::TrustHost {
            host: self.host_label(),
            algorithm: key.algorithm().as_str().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            reply,
        });
        let trusted = asked.is_ok() && answer.await.unwrap_or(false);
        if !trusted {
            return Ok(self.reject(format!(
                "the host key of {} was not trusted",
                self.host_label()
            )));
        }

        if let Some(home) = std::env::home_dir() {
            let _ = std::fs::create_dir_all(home.join(".ssh"));
        }
        // Remembering is a convenience; a failure here should not block the deploy.
        let _ = russh::keys::known_hosts::learn_known_hosts(&self.host, self.port, key);
        Ok(true)
    }
}
