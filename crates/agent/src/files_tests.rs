use std::time::{SystemTime, UNIX_EPOCH};

use shipdeck_protocol::message::agent::{HashCurrent, InspectWork};

use super::{hash_current, inspect_work};

#[tokio::test]
async fn hashes_files_and_rejects_unsafe_paths() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("shipdeck-hash-{}-{suffix}", std::process::id()));
    let current = root.join("current");
    tokio::fs::create_dir_all(current.join("nested"))
        .await
        .expect("create directory");
    tokio::fs::write(current.join("nested/file"), b"abc")
        .await
        .expect("write file");
    #[cfg(unix)]
    std::os::unix::fs::symlink("nested", current.join("link")).expect("create symlink");
    let request = HashCurrent {
        root: root.to_string_lossy().into_owned(),
        paths: vec!["nested/file".into(), "missing".into(), "link/file".into()],
    };
    let result = hash_current(&request).await.expect("hash files");
    assert_eq!(result.hashes.len(), 1);
    assert_eq!(
        result.hashes.get("nested/file").map(String::as_str),
        Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    let unsafe_request = HashCurrent {
        root: request.root,
        paths: vec!["../outside".into()],
    };
    assert!(hash_current(&unsafe_request).await.is_err());
    tokio::fs::remove_dir_all(root)
        .await
        .expect("remove test folder");
}

#[cfg(unix)]
#[tokio::test]
async fn inspection_reports_partial_files_without_removing_them() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("shipdeck-inspect-{}-{suffix}", std::process::id()));
    tokio::fs::create_dir_all(root.join("releases/v1"))
        .await
        .expect("releases");
    std::os::unix::fs::symlink("releases/v1", root.join("current")).expect("live link");
    tokio::fs::write(root.join("releases/.v2.upload.partial"), b"unfinished")
        .await
        .expect("partial");
    let result = inspect_work(InspectWork {
        root: root.to_string_lossy().into_owned(),
    })
    .await
    .expect("inspect");
    assert_eq!(result.current.as_deref(), Some("v1"));
    assert_eq!(result.unfinished, ["releases/.v2.upload.partial"]);
    assert!(root.join("releases/.v2.upload.partial").exists());
    tokio::fs::remove_dir_all(root)
        .await
        .expect("remove test folder");
}
