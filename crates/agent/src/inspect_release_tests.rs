use std::time::{SystemTime, UNIX_EPOCH};

use shipdeck_protocol::message::agent::{
    AgentDropRequest, AgentFileKind, AgentReleaseContentsRequest, AgentReleaseReadRequest,
};

use super::{contents, drop_one, read};

fn folder(name: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("shipdeck-{name}-{}-{suffix}", std::process::id()))
}

#[tokio::test]
async fn lists_a_folder_and_reads_a_file() {
    let root = folder("inspect-release");
    std::fs::create_dir_all(root.join("releases/v1/dir")).expect("release");
    std::fs::create_dir_all(root.join("releases/v1/.shipdeck")).expect("record folder");
    std::fs::write(root.join("releases/v1/dir/file"), "hello").expect("file");
    std::fs::write(
        root.join("releases/v1/.shipdeck/release.json"),
        "{\"version\":\"1\"}",
    )
    .expect("manifest");
    #[cfg(unix)]
    std::os::unix::fs::symlink("dir/file", root.join("releases/v1/link")).expect("link");
    let found = contents(AgentReleaseContentsRequest {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        backup: false,
    })
    .await
    .expect("contents");
    assert!(found.manifest.is_some());
    let link = found.files.iter().find(|f| f.path == "link").expect("link");
    assert_eq!(link.kind, AgentFileKind::Link);
    assert_eq!(link.target.as_deref(), Some("dir/file"));
    let text = read(AgentReleaseReadRequest {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        backup: false,
        path: "dir/file".into(),
        cap: 512 * 1024,
    })
    .await
    .expect("read");
    assert_eq!(text.text, "hello");
    assert!(!text.binary);
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn reads_a_backup_and_drops_one_kind_at_a_time() {
    let root = folder("inspect-backup");
    std::fs::create_dir_all(root.join("releases/v1")).expect("release");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    std::fs::write(root.join("releases/v1/keep"), "live").expect("file");
    // Make a backup with the agent's own packer.
    let request = shipdeck_protocol::message::agent::AgentBackupCreate {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        paths: vec!["keep".into()],
    };
    crate::backup::create(request).await.expect("backup");
    let text = read(AgentReleaseReadRequest {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        backup: true,
        path: "keep".into(),
        cap: 512 * 1024,
    })
    .await
    .expect("read backup");
    assert_eq!(text.text, "live");
    drop_one(AgentDropRequest {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        open: false,
        backup: true,
    })
    .await
    .expect("drop backup");
    assert!(root.join("releases/v1").is_dir());
    assert!(!root.join("backups/v1.tar.zst").exists());
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn dropping_refuses_the_live_release() {
    let root = folder("drop-live");
    std::fs::create_dir_all(root.join("releases/v1")).expect("release");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    #[cfg(unix)]
    std::os::unix::fs::symlink("releases/v1", root.join("current")).expect("link");
    let result = drop_one(AgentDropRequest {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        open: true,
        backup: false,
    })
    .await;
    assert!(result.is_err());
    assert!(root.join("releases/v1").is_dir());
    std::fs::remove_dir_all(root).expect("remove");
}
