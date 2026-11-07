#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use super::{private_checks, temp_dir_script};
use crate::ssh::sh_quote;

fn run(script: &str) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .expect("sh runs")
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "shipdeck-install-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("the scratch folder");
    path
}

fn checks_for(path: &Path) -> String {
    format!(
        "set -e\n{}",
        private_checks(&sh_quote(&path.to_string_lossy()))
    )
}

#[test]
fn a_private_owned_folder_passes() {
    let path = scratch("ok");
    std::fs::set_permissions(&path, PermissionsExt::from_mode(0o700)).expect("mode");
    let output = run(&checks_for(&path));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(&path).expect("cleanup");
}

#[test]
fn a_symlink_is_refused() {
    let target = scratch("link-target");
    std::fs::set_permissions(&target, PermissionsExt::from_mode(0o700)).expect("mode");
    let link = target.with_extension("link");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let output = run(&checks_for(&link));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("symlink"));
    std::fs::remove_file(&link).expect("cleanup link");
    std::fs::remove_dir_all(&target).expect("cleanup target");
}

#[test]
fn a_folder_open_to_others_is_refused() {
    let path = scratch("open");
    std::fs::set_permissions(&path, PermissionsExt::from_mode(0o777)).expect("mode");
    let output = run(&checks_for(&path));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not private"));
    std::fs::remove_dir_all(&path).expect("cleanup");
}

#[test]
fn temp_folders_are_unique_and_private() {
    let first = run(&temp_dir_script());
    let second = run(&temp_dir_script());
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let first = String::from_utf8(first.stdout).expect("a path");
    let second = String::from_utf8(second.stdout).expect("a path");
    assert_ne!(first, second);
    for folder in [&first, &second] {
        let meta = std::fs::metadata(folder).expect("the folder exists");
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        assert!(!meta.file_type().is_symlink());
        std::fs::remove_dir_all(folder).expect("cleanup");
    }
}
