use std::time::{SystemTime, UNIX_EPOCH};

use shipdeck_protocol::message::agent::ProjectLock;

use super::ProjectLocks;

#[test]
fn project_lock_blocks_other_sessions_and_is_released_on_drop() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("shipdeck-lock-{}-{suffix}", std::process::id()));
    std::fs::create_dir(&root).expect("directory");
    let request = ProjectLock {
        root: root.to_string_lossy().into_owned(),
        token: 1,
    };
    let mut first = ProjectLocks::default();
    let mut second = ProjectLocks::default();
    first.acquire(&request).expect("lock project");
    let wrong = ProjectLock {
        root: request.root.clone(),
        token: 2,
    };
    assert!(first.release(&wrong).is_err());
    assert!(first.acquire(&request).is_err());
    assert!(second.acquire(&request).is_err());
    drop(first);
    second.acquire(&request).expect("lock after disconnect");
    second.release(&request).expect("unlock");
    assert!(second.release(&request).is_err());
    std::fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn holds_reports_only_a_held_project() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("shipdeck-holds-{}-{suffix}", std::process::id()));
    std::fs::create_dir(&root).expect("directory");
    let mut locks = ProjectLocks::default();
    assert!(!locks.holds(&root.to_string_lossy()));
    locks
        .acquire(&ProjectLock {
            root: root.to_string_lossy().into_owned(),
            token: 1,
        })
        .expect("lock");
    assert!(locks.holds(&root.to_string_lossy()));
    std::fs::remove_dir_all(root).expect("remove");
}
