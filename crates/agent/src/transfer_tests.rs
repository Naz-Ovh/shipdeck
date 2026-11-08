use std::io::Write as _;
use std::path::Path;

use sha2::{Digest, Sha256};
use shipdeck_protocol::message::agent::UploadPrepare;

use super::{prepare, private_folder, receive};

#[tokio::test]
async fn transfer_checks_length_checksum_and_one_use() {
    let root = private_folder().await.expect("test directory");
    tokio::fs::create_dir(root.join("releases"))
        .await
        .expect("releases");
    let mut request = UploadPrepare {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        bytes: 3,
        sha256: base16ct::lower::encode_string(&Sha256::digest(b"abc")),
        lock_token: 1,
        retry: false,
    };
    let mut lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(".shipdeck.lock"))
        .expect("lock file");
    writeln!(lock, "1").expect("token");
    lock.try_lock().expect("hold lock");
    for bytes in [b"ab".as_slice(), b"abcd".as_slice(), b"xyz".as_slice()] {
        let ticket = prepare(&request).await.expect("reserve");
        assert!(receive(&ticket.reservation, bytes).await.is_err());
        assert!(!root.join("releases/.v1.tar.zst").exists());
        assert!(
            receive(&ticket.reservation, b"abc".as_slice())
                .await
                .is_err()
        );
        if !request.retry {
            assert!(prepare(&request).await.is_err());
            request.retry = true;
        }
        let folder = Path::new(&ticket.reservation)
            .parent()
            .expect("ticket folder");
        tokio::fs::remove_dir_all(folder)
            .await
            .expect("remove ticket");
    }
    let ticket = prepare(&request).await.expect("reserve");
    receive(&ticket.reservation, b"abc".as_slice())
        .await
        .expect("upload");
    assert_eq!(
        tokio::fs::read(root.join("releases/.v1.tar.zst"))
            .await
            .expect("read upload"),
        b"abc"
    );
    assert!(prepare(&request).await.is_err());
    let folder = Path::new(&ticket.reservation)
        .parent()
        .expect("ticket folder");
    tokio::fs::remove_dir_all(folder)
        .await
        .expect("remove ticket");
    tokio::fs::remove_dir_all(root)
        .await
        .expect("remove test directory");
}

#[tokio::test]
async fn an_upload_ticket_stops_working_when_its_lock_is_released() {
    let root = private_folder().await.expect("test directory");
    tokio::fs::create_dir(root.join("releases"))
        .await
        .expect("releases");
    let request = UploadPrepare {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        bytes: 3,
        sha256: base16ct::lower::encode_string(&Sha256::digest(b"abc")),
        lock_token: 1,
        retry: false,
    };
    let mut lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(".shipdeck.lock"))
        .expect("lock file");
    writeln!(lock, "1").expect("token");
    lock.try_lock().expect("hold lock");
    let ticket = prepare(&request).await.expect("reserve");
    drop(lock);
    assert!(
        receive(&ticket.reservation, b"abc".as_slice())
            .await
            .is_err()
    );
    assert!(!root.join("releases/.v1.tar.zst").exists());
    let folder = Path::new(&ticket.reservation)
        .parent()
        .expect("ticket folder");
    tokio::fs::remove_dir_all(folder)
        .await
        .expect("remove ticket");
    tokio::fs::remove_dir_all(root)
        .await
        .expect("remove test directory");
}
