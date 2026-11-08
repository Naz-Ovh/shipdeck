//! Small helpers shared by the agent unit tests.

use std::path::Path;
use std::time::{Duration, Instant};

/// Waits until a project lock file can be locked.
///
/// Tests run in parallel. A short-lived child process started by a sibling
/// test inherits this test's descriptors between fork and exec, which keeps
/// the lock held for that moment. Waiting here tests the lock contract, not
/// that fork window.
pub(crate) fn wait_until_unlocked(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("opening the project lock");
        match file.try_lock() {
            Ok(()) => return,
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(err)) => {
                panic!("checking the project lock: {err}");
            }
        }
        assert!(
            Instant::now() < deadline,
            "the project lock was not released"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
