use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;
use crate::remote::sudo_commands;

/// The script as committed at the repository root.
const SCRIPT: &str = include_str!("../../../server-setup.sh");

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server-setup.sh")
}

#[test]
fn url_is_pinned_to_this_release() {
    assert_eq!(
        script_url(),
        format!(
            "https://raw.githubusercontent.com/Naz-Ovh/shipdeck/v{}/server-setup.sh",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn script_runs_exactly_the_sudo_commands_shipdeck_checks() {
    let start = SCRIPT
        .find("SUDO_COMMANDS=(\n")
        .expect("SUDO_COMMANDS in the script");
    let body = &SCRIPT[start + "SUDO_COMMANDS=(\n".len()..];
    let listed: Vec<&str> = body[..body.find("\n)").unwrap()]
        .lines()
        .map(|l| l.trim().trim_matches('"'))
        .collect();
    let expected = sudo_commands("$APP_DIR", "$SERVICE");
    assert_eq!(
        listed, expected,
        "server-setup.sh and remote::sudo_commands differ"
    );
}

#[test]
fn script_names_the_sudoers_file_without_dots() {
    // sudo skips files in /etc/sudoers.d whose names contain a `.`.
    assert!(SCRIPT.contains(r#"SUDOERS_FILE="/etc/sudoers.d/shipdeck-${SERVICE//./_}""#));
}

#[test]
fn script_is_valid_bash() {
    let status = Command::new("bash")
        .arg("-n")
        .arg(script_path())
        .status()
        .unwrap();
    assert!(status.success());
}

/// Runs the script with `args`; returns its exit code and stderr.
fn run_script(args: &[String]) -> (i32, String) {
    let out = Command::new("bash")
        .arg(script_path())
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn script_refuses_unsafe_values_before_touching_anything() {
    let base = |user: &str, dir: &str, service: &str| {
        vec![
            "--user".into(),
            user.into(),
            "--dir".into(),
            dir.into(),
            "--service".into(),
            service.into(),
        ]
    };
    for (args, why) in [
        (base("a b", "/opt/x", "x"), "--user"),
        (base("deploy", "opt/x", "x"), "--dir"),
        (base("deploy", "/opt/../etc", "x"), "'..'"),
        (base("deploy", "/opt/x", "x;id"), "--service"),
        (base("deploy", "/opt/x", "x.service"), ".service"),
        (
            [
                base("deploy", "/opt/x", "x"),
                vec!["--manager".into(), "pm2;id".into()],
            ]
            .concat(),
            "--manager",
        ),
        (vec!["--user".into()], "needs a value"),
        (vec!["--bogus".into()], "unknown argument"),
    ] {
        let (code, stderr) = run_script(&args);
        assert_ne!(code, 0, "{args:?}");
        assert!(stderr.contains(why), "{args:?}: {stderr}");
    }
    let mut multi_line_key = base("deploy", "/opt/x", "x");
    multi_line_key.extend(["--key".into(), "ssh-ed25519 AAAA a\nssh-rsa BBBB b".into()]);
    assert!(run_script(&multi_line_key).1.contains("single line"));
}

#[test]
fn script_accepts_valid_values() {
    // Past validation the script needs root; as root this test would set up
    // the machine it runs on, so it only runs as a normal user.
    let uid = Command::new("id").arg("-u").output().unwrap();
    if String::from_utf8_lossy(&uid.stdout).trim() == "0" {
        return;
    }
    for (service, manager) in [
        ("my-api", "systemd"),
        ("my.api", "systemd"),
        ("worker@1", "systemd"),
        ("my-api", "pm2"),
    ] {
        let args: Vec<String> = [
            "--user",
            "deploy",
            "--dir",
            "/opt/my-api",
            "--service",
            service,
            "--manager",
            manager,
            "--key",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPR1CM5+/x9= it's me@laptop",
        ]
        .map(String::from)
        .to_vec();
        let (code, stderr) = run_script(&args);
        assert_ne!(code, 0);
        assert!(stderr.contains("run this as root"), "{service}: {stderr}");
    }
}
