use std::time::{SystemTime, UNIX_EPOCH};

use shipdeck_protocol::message::agent::{AgentCleanupWork, AgentDiscard};

use super::{cleanup, discard};

fn folder(name: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("shipdeck-{name}-{}-{suffix}", std::process::id()))
}

#[tokio::test]
async fn removes_only_the_approved_artifacts() {
    let root = folder("cleanup");
    std::fs::create_dir_all(root.join("releases/v1")).expect("release");
    std::fs::create_dir_all(root.join("releases/.v2.partial")).expect("partial");
    std::fs::write(root.join("releases/.v2.tar.zst"), "archive").expect("archive");
    std::fs::write(root.join(".current-switch-abc.partial"), "link").expect("switch");
    let removed = cleanup(AgentCleanupWork {
        root: root.to_string_lossy().into_owned(),
        artifacts: vec![
            "releases/.v2.partial".into(),
            "releases/.v2.tar.zst".into(),
            ".current-switch-abc.partial".into(),
        ],
    })
    .await
    .expect("cleanup");
    assert_eq!(removed.removed.len(), 3);
    assert!(!root.join("releases/.v2.partial").exists());
    assert!(!root.join("releases/.v2.tar.zst").exists());
    assert!(!root.join(".current-switch-abc.partial").exists());
    assert!(root.join("releases/v1").is_dir());
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn refuses_a_live_release_and_paths_outside_the_project() {
    let root = folder("cleanup-refuse");
    std::fs::create_dir_all(root.join("releases/v1")).expect("release");
    for artifact in [
        "releases/v1",
        "../outside",
        "current",
        "releases/../backups/x",
    ] {
        let result = cleanup(AgentCleanupWork {
            root: root.to_string_lossy().into_owned(),
            artifacts: vec![artifact.into()],
        })
        .await;
        assert!(result.is_err(), "{artifact} was accepted");
    }
    assert!(root.join("releases/v1").is_dir());
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn discard_removes_one_release_and_its_artifacts() {
    let root = folder("discard");
    std::fs::create_dir_all(root.join("releases/v1/nested")).expect("release");
    std::fs::create_dir_all(root.join("releases/v2")).expect("other release");
    std::fs::create_dir_all(root.join("releases/.v1.partial")).expect("partial");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    std::fs::write(root.join("releases/.v1.tar.zst"), "archive").expect("archive");
    std::fs::write(root.join("backups/v1.tar.zst"), "zip").expect("backup");
    std::fs::write(root.join("backups/.v1.files"), "list").expect("list");
    discard(AgentDiscard {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
    })
    .await
    .expect("discard");
    assert!(!root.join("releases/v1").exists());
    assert!(!root.join("backups/v1.tar.zst").exists());
    assert!(!root.join("releases/.v1.tar.zst").exists());
    assert!(!root.join("releases/.v1.partial").exists());
    assert!(!root.join("backups/.v1.files").exists());
    assert!(root.join("releases/v2").is_dir());
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn discard_refuses_the_live_release() {
    let root = folder("discard-live");
    std::fs::create_dir_all(root.join("releases/v1")).expect("release");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    #[cfg(unix)]
    std::os::unix::fs::symlink("releases/v1", root.join("current")).expect("link");
    let result = discard(AgentDiscard {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
    })
    .await;
    assert!(result.is_err());
    assert!(root.join("releases/v1").is_dir());
    std::fs::remove_dir_all(root).expect("remove");
}

#[cfg(unix)]
#[tokio::test]
async fn cleanup_refuses_a_linked_project_folder() {
    let root = folder("cleanup-link");
    let elsewhere = folder("cleanup-elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("elsewhere");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(elsewhere.join(".v1.partial"), "kept").expect("partial");
    std::os::unix::fs::symlink(&elsewhere, root.join("releases")).expect("link");
    let result = cleanup(AgentCleanupWork {
        root: root.to_string_lossy().into_owned(),
        artifacts: vec!["releases/.v1.partial".into()],
    })
    .await;
    assert!(result.is_err());
    assert!(elsewhere.join(".v1.partial").exists());
    std::fs::remove_dir_all(root).expect("remove");
    std::fs::remove_dir_all(elsewhere).expect("remove");
}
