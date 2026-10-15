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
