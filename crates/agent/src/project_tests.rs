#![cfg(unix)]

use std::time::{SystemTime, UNIX_EPOCH};

use shipdeck_protocol::message::agent::{InspectWork, SwitchRelease};

use super::{prepare, switch_link, validate_release};

#[tokio::test]
async fn switch_changes_only_to_an_existing_release() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("shipdeck-switch-{}-{suffix}", std::process::id()));
    tokio::fs::create_dir(&root).await.expect("root");
    let root_text = root.to_string_lossy().into_owned();
    prepare(InspectWork {
        root: root_text.clone(),
    })
    .await
    .expect("prepare");
    tokio::fs::create_dir(root.join("releases/v1"))
        .await
        .expect("release");
    let request = SwitchRelease {
        root: root_text.clone(),
        release: "v1".into(),
    };
    validate_release(&request).await.expect("valid release");
    switch_link(&request).await.expect("switch");
    assert_eq!(
        tokio::fs::read_link(root.join("current"))
            .await
            .expect("current"),
        std::path::Path::new("releases/v1")
    );
    let invalid = SwitchRelease {
        root: root_text,
        release: "../outside".into(),
    };
    assert!(validate_release(&invalid).await.is_err());
    assert_eq!(
        tokio::fs::read_link(root.join("current"))
            .await
            .expect("unchanged"),
        std::path::Path::new("releases/v1")
    );
    tokio::fs::remove_dir_all(root)
        .await
        .expect("remove test folder");
}
