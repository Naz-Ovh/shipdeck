use std::time::{SystemTime, UNIX_EPOCH};

use shipdeck_protocol::message::agent::{AgentBackupCreate, AgentBackupRestore};

use super::{create, restore};

fn folder(name: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("shipdeck-{name}-{}-{suffix}", std::process::id()))
}

#[tokio::test]
async fn packs_and_restores_selected_paths() {
    let root = folder("backup");
    std::fs::create_dir_all(root.join("releases/v1/dir")).expect("release");
    std::fs::write(root.join("releases/v1/dir/file"), "kept").expect("file");
    std::fs::write(root.join("releases/v1/other"), "left out").expect("file");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    let created = create(AgentBackupCreate {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        paths: vec!["dir".into(), "dir/file".into()],
    })
    .await
    .expect("create");
    assert!(created.bytes > 0);
    assert!(root.join("backups/v1.tar.zst").is_file());
    std::fs::remove_dir_all(root.join("releases/v1")).expect("remove release");
    restore(AgentBackupRestore {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
    })
    .await
    .expect("restore");
    assert_eq!(
        std::fs::read_to_string(root.join("releases/v1/dir/file")).expect("file"),
        "kept"
    );
    assert!(!root.join("releases/v1/other").exists());
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn refuses_an_unfinished_restore_until_it_is_cleaned() {
    let root = folder("backup-restore");
    std::fs::create_dir_all(root.join("releases/.v1.restore.partial")).expect("partial");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    std::fs::write(root.join("backups/v1.tar.zst"), "not read").expect("backup");
    let result = restore(AgentBackupRestore {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
    })
    .await;
    assert!(result.is_err());
    assert!(root.join("releases/.v1.restore.partial").is_dir());
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn refuses_paths_that_escape_the_release() {
    let root = folder("backup-escape");
    std::fs::create_dir_all(root.join("releases/v1")).expect("release");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    let result = create(AgentBackupCreate {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        paths: vec!["../../etc/passwd".into()],
    })
    .await;
    assert!(result.is_err());
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn an_unfinished_backup_is_not_overwritten() {
    let root = folder("backup-partial");
    std::fs::create_dir_all(root.join("releases/v1")).expect("release");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    std::fs::write(root.join("releases/v1/file"), "data").expect("file");
    std::fs::write(root.join("backups/.v1.old.part"), "unfinished").expect("partial");
    let result = create(AgentBackupCreate {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        paths: vec!["file".into()],
    })
    .await;
    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("backups/.v1.old.part")).expect("partial"),
        "unfinished"
    );
    std::fs::remove_dir_all(root).expect("remove");
}

#[cfg(unix)]
#[tokio::test]
async fn a_symbolic_link_is_not_followed_as_a_backup() {
    let root = folder("backup-link");
    std::fs::create_dir_all(root.join("releases/v1")).expect("release");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    std::fs::write(root.join("releases/v1/file"), "data").expect("file");
    std::fs::write(root.join("target"), "keep me").expect("target");
    std::os::unix::fs::symlink(root.join("target"), root.join("backups/v1.tar.zst")).expect("link");
    let result = create(AgentBackupCreate {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
        paths: vec!["file".into()],
    })
    .await;
    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("target")).expect("target"),
        "keep me"
    );
    std::fs::remove_dir_all(root).expect("remove");
}

#[test]
fn a_failed_publish_keeps_the_packed_backup() {
    let root = folder("backup-publish");
    std::fs::create_dir_all(root.join("done")).expect("done");
    let part = root.join("part");
    std::fs::write(&part, "packed").expect("part");
    let missing = root.join("missing").join("backup.tar.zst");
    let result = super::publish_backup(&part, &missing, "v1");
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&part).expect("kept part"), "packed");
    std::fs::remove_dir_all(root).expect("remove");
}
