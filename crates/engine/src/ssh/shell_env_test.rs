use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

/// A home folder whose `.bashrc` stops early unless interactive, as
/// Ubuntu's does, and sets a tool's PATH after that, as nvm does. It also
/// prints and reads, which must not get in the way.
fn home(name: &str, bashrc_tail: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-shellenv-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tool/bin")).unwrap();
    std::fs::write(
        dir.join(".profile"),
        "if [ -f \"$HOME/.bashrc\" ]; then . \"$HOME/.bashrc\"; fi\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(".bashrc"),
        format!("[ -z \"$PS1\" ] && return\necho 'welcome!'\nread -r answer\n{bashrc_tail}\n"),
    )
    .unwrap();
    dir
}

/// Runs `script` in `sh` the way a session does, as the user of `home`.
fn sh(home: &Path, script: &str) -> std::process::Output {
    Command::new("sh")
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("HOME", home)
        .env("SHELL", "bash")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap()
}

#[test]
fn a_command_sees_what_the_interactive_startup_files_set() {
    let home = home(
        "path",
        "export PATH=\"$HOME/tool/bin:$PATH\"\nexport ODD=\"it's \\\"quoted\\\"\n and two lines\"",
    );
    let captured = sh(&home, CAPTURE);
    let stdout = String::from_utf8(captured.stdout).unwrap();
    let prelude = prelude(captured.status.code().unwrap() as u32, &stdout).unwrap();
    assert!(!stdout.contains("welcome!"));

    // Without it the tool is not on PATH; with it, it is, and an awkward
    // value comes through as it was.
    let plain = sh(&home, "printf %s \"$PATH\"");
    assert!(!String::from_utf8_lossy(&plain.stdout).contains("tool/bin"));
    let after = sh(
        &home,
        &format!("{prelude}printf '%s|%s' \"$PATH\" \"$ODD\""),
    );
    let after = String::from_utf8(after.stdout).unwrap();
    let (path, odd) = after.split_once('|').unwrap();
    assert!(
        path.contains(&format!("{}/tool/bin", home.display())),
        "{path}"
    );
    assert_eq!(odd, "it's \"quoted\"\n and two lines");
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_failing_startup_is_reported_not_used() {
    let home = home("fails", "exit 3");
    let captured = sh(&home, CAPTURE);
    let stdout = String::from_utf8(captured.stdout).unwrap();
    let err = prelude(captured.status.code().unwrap() as u32, &stdout).unwrap_err();
    assert!(err.contains("failed (exit 3)"), "{err}");
    assert_eq!(
        prelude(0, "  \n").unwrap_err(),
        "the login shell exported nothing"
    );
    std::fs::remove_dir_all(home).unwrap();
}
