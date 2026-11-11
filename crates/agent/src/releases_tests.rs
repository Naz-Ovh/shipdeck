use std::time::{SystemTime, UNIX_EPOCH};

use shipdeck_protocol::message::agent::AgentReleaseKind;

use super::{files, list};
use shipdeck_protocol::message::agent::AgentReleaseFilesRequest;

fn folder(name: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("shipdeck-{name}-{}-{suffix}", std::process::id()))
}

#[tokio::test]
async fn lists_open_releases_and_backups() {
    let root = folder("releases");
    std::fs::create_dir_all(root.join("releases/v1/nested")).expect("release");
    std::fs::create_dir_all(root.join("releases/v1/.shipdeck")).expect("settings");
    std::fs::write(
        root.join("releases/v1/.shipdeck/release.json"),
        "{\"version\":\"1\"}",
    )
    .expect("manifest");
    std::fs::write(root.join("releases/v1/nested/file"), "abc").expect("file");
    std::fs::create_dir_all(root.join("backups")).expect("backups");
    std::fs::write(root.join("backups/v1.tar.zst"), "zip").expect("backup");
    std::fs::write(root.join("backups/v2.tar.zst"), "zip").expect("backup");
    #[cfg(unix)]
    std::os::unix::fs::symlink("releases/v1", root.join("current")).expect("link");
    let listing = list(root.to_string_lossy().into_owned())
        .await
        .expect("list");
    assert_eq!(listing.current.as_deref(), Some("v1"));
    let v1 = listing.entries.iter().find(|e| e.id == "v1").expect("v1");
    assert_eq!(v1.kind, AgentReleaseKind::Open);
    assert!(v1.current);
    assert!(v1.backup);
    assert!(v1.manifest.is_some());
    assert_eq!(v1.size_bytes, Some(3 + 15));
    let v2 = listing.entries.iter().find(|e| e.id == "v2").expect("v2");
    assert_eq!(v2.kind, AgentReleaseKind::Compressed);
    assert!(!v2.current);
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn lists_release_files_without_following_links() {
    let root = folder("release-files");
    std::fs::create_dir_all(root.join("releases/v1/dir")).expect("release");
    std::fs::write(root.join("releases/v1/dir/file"), "abc").expect("file");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc", root.join("releases/v1/out")).expect("link");
    let result = files(AgentReleaseFilesRequest {
        root: root.to_string_lossy().into_owned(),
        release: "v1".into(),
    })
    .await
    .expect("files");
    let paths: Vec<&str> = result.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"dir"));
    assert!(paths.contains(&"dir/file"));
    assert!(paths.contains(&"out"));
    assert!(!paths.contains(&"out/passwd"));
    std::fs::remove_dir_all(root).expect("remove");
}

#[tokio::test]
async fn an_empty_project_has_no_releases() {
    let root = folder("empty-releases");
    std::fs::create_dir_all(&root).expect("root");
    let listing = list(root.to_string_lossy().into_owned())
        .await
        .expect("empty list");
    assert!(listing.current.is_none());
    assert_eq!(listing.entries.len(), 0);
    std::fs::remove_dir_all(root).expect("remove");
}
