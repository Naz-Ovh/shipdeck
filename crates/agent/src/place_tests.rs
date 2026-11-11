use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{append_line, private_dir, state_file};

fn scratch(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "shipdeck-place-{name}-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).expect("test directory");
    dir
}

#[test]
fn each_user_has_their_own_state_file() {
    let root = Path::new("/var/lib/shipdeck");
    assert_eq!(
        state_file(root, "deploy").unwrap(),
        Path::new("/var/lib/shipdeck/users/deploy/state.db")
    );
    for bad in ["", ".", "..", "a/b"] {
        assert!(state_file(root, bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_private_folder_is_made_once_and_never_a_link() {
    let dir = scratch("dir");
    let folder = dir.join("users");
    private_dir(&folder).unwrap();
    private_dir(&folder).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let link = dir.join("link");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        assert!(private_dir(&link).is_err());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_log_appends_lines_and_refuses_a_link() {
    let dir = scratch("log");
    let log = dir.join("agent.log");
    append_line(&log, "first\n").unwrap();
    append_line(&log, "second").unwrap();
    let text = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].ends_with(" first") && lines[1].ends_with(" second"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let link = dir.join("linked.log");
        std::os::unix::fs::symlink(&log, &link).unwrap();
        assert!(append_line(&link, "via a link").is_err());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
