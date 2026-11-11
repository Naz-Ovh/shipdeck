use std::process::Command;

use super::*;

/// A fresh ed25519 key from ssh-keygen, with `passphrase` (empty for none):
/// its private text and its public line. `None` without ssh-keygen.
fn generated(name: &str, passphrase: &str) -> Option<(String, String)> {
    let dir = std::env::temp_dir().join(format!("shipdeck-login-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("id");
    let made = Command::new("ssh-keygen")
        .args([
            "-q", "-t", "ed25519", "-C", "test@box", "-N", passphrase, "-f",
        ])
        .arg(&path)
        .status()
        .ok()?;
    assert!(made.success());
    let private = std::fs::read_to_string(&path).unwrap();
    let public = std::fs::read_to_string(dir.join("id.pub")).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    Some((private, public.trim().to_string()))
}

#[tokio::test]
async fn a_key_reads_from_its_text_and_a_given_passphrase_unlocks_it() {
    let Some((plain, _)) = generated("plain", "") else {
        return;
    };
    let (asker, _questions) = tokio::sync::mpsc::unbounded_channel();
    let secrets = Secrets::default();
    load_key(&KeySource::text(&plain), &asker, &secrets)
        .await
        .unwrap();

    let (locked, _) = generated("locked", "s3cret passphrase").unwrap();
    let source = KeySource::text(&locked);
    secrets.give_passphrase("wrong");
    let err = load_key(&source, &asker, &secrets).await.unwrap_err();
    assert!(
        err.to_string().contains("does not unlock the key text"),
        "{err}"
    );
    secrets.give_passphrase("s3cret passphrase");
    load_key(&source, &asker, &secrets).await.unwrap();

    let err = load_key(&KeySource::text("not a key"), &asker, &secrets)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("reading the key the key text"),
        "{err}"
    );
}

#[test]
fn login_never_prints_a_secret() {
    let shown = format!(
        "{:?} {:?}",
        Login::KeyText("-----BEGIN OPENSSH PRIVATE KEY-----".into()),
        Login::Password("hunter2".into())
    );
    assert_eq!(shown, "key text password");
}

/// A server that lets anyone in with no secret, or no one.
struct NoneServer {
    open: bool,
}

impl russh::server::Handler for NoneServer {
    type Error = russh::Error;

    fn auth_none(
        &mut self,
        _user: &str,
    ) -> impl Future<Output = Result<russh::server::Auth, Self::Error>> + Send {
        std::future::ready(Ok(if self.open {
            russh::server::Auth::Accept
        } else {
            russh::server::Auth::reject()
        }))
    }
}

/// A client that trusts the test server's fresh host key.
struct TrustingClient;

impl russh::client::Handler for TrustingClient {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(true))
    }
}

/// Whether a `none` login gets in, against a server that is `open` or not.
/// `None` without ssh-keygen.
async fn none_login(open: bool) -> Option<bool> {
    let (host_key, _) = generated(&format!("host-{open}"), "")?;
    let config = russh::server::Config {
        keys: vec![decode_secret_key(&host_key, None).unwrap()],
        auth_rejection_time: std::time::Duration::from_millis(1),
        ..Default::default()
    };
    let (client_side, server_side) = tokio::io::duplex(64 * 1024);
    let server = russh::server::run_stream(Arc::new(config), server_side, NoneServer { open });
    tokio::spawn(async move { server.await.map(|_running| ()) });
    let mut handle = russh::client::connect_stream(
        Arc::new(russh::client::Config::default()),
        client_side,
        TrustingClient,
    )
    .await
    .unwrap();
    Some(none_accepted(&mut handle, "box").await.unwrap())
}

#[tokio::test]
async fn a_passwordless_server_lets_the_none_login_in() {
    if let Some(accepted) = none_login(true).await {
        assert!(accepted);
    }
}

#[tokio::test]
async fn a_server_that_needs_a_secret_refuses_the_none_login() {
    if let Some(accepted) = none_login(false).await {
        assert!(!accepted);
    }
}
