use std::process::Command;

use super::*;
use crate::config::DeployConfig;
use crate::remote::sudo_commands;
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

/// The script as committed at the repository root.
const SCRIPT: &str = include_str!("../../../server-setup.sh");

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server-setup.sh")
}

fn deployable(service: &str, identity: Option<PathBuf>) -> Deployable {
    let config: DeployConfig = serde_json::from_value(serde_json::json!({
        "name": "My API (prod)",
        "ssh": { "host": "prod", "user": "deploy" },
        "service": { "name": service, "dir": "/opt/my-api" },
        "pipeline": { "pack": { "binary": "target/release/my-api" } },
    }))
    .unwrap();
    let found = HostConfig {
        host_name: Some("203.0.113.10".into()),
        user: Some("root".into()),
        identity_file: identity,
        ..HostConfig::default()
    };
    let endpoint = Endpoint::from_config(&config.ssh, found).unwrap();
    Deployable {
        folder: PathBuf::from("./deploy/api-prod"),
        slug: "api-prod".into(),
        config,
        endpoint,
    }
}

fn key_pair(name: &str, public: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shipdeck-setup-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("id_deploy"), "not a real key").unwrap();
    std::fs::write(dir.join("id_deploy.pub"), format!("\n{public}\n")).unwrap();
    dir.join("id_deploy")
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
fn command_carries_the_deployable_and_quotes_the_key() {
    let key = key_pair("cmd", "ssh-ed25519 AAAAC3Nza me@laptop");
    let setup = command(&deployable("my-api", Some(key.clone())), &Login::Auto);
    assert_eq!(
        setup.command,
        format!(
            "curl -fsSL {} | bash -s -- --user deploy --dir /opt/my-api --service my-api \
             --key 'ssh-ed25519 AAAAC3Nza me@laptop'",
            script_url()
        )
    );
    assert_eq!(setup.key, Some(key.clone()));
    std::fs::remove_dir_all(key.parent().unwrap()).unwrap();
}

#[test]
fn command_without_a_key_leaves_it_out() {
    let setup = command(&deployable("my-api", None), &Login::Auto);
    assert!(!setup.command.contains("--key"));
    assert_eq!(setup.key, None);
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
fn script_names_the_sudoers_file_like_shipdeck() {
    assert!(SCRIPT.contains(r#"SUDOERS_FILE="/etc/sudoers.d/shipdeck-${SERVICE//./_}""#));
    assert_eq!(sudoers_file("my.app"), "/etc/sudoers.d/shipdeck-my_app");
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
fn script_accepts_what_shipdeck_passes() {
    // Past validation the script needs root; as root this test would set up
    // the machine it runs on, so it only runs as a normal user.
    let uid = Command::new("id").arg("-u").output().unwrap();
    if String::from_utf8_lossy(&uid.stdout).trim() == "0" {
        return;
    }
    for service in ["my-api", "my.api", "worker@1"] {
        let args = arguments(
            &deployable(service, None),
            Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPR1CM5+/x9= it's me@laptop"),
        );
        let (code, stderr) = run_script(&args);
        assert_ne!(code, 0);
        assert!(stderr.contains("run this as root"), "{service}: {stderr}");
    }
}

#[test]
fn reads_the_public_key_file_next_to_the_private_key() {
    let key = key_pair("pub", "ssh-ed25519 AAAAC3Nza test@box");
    assert_eq!(
        public_key(&key).as_deref(),
        Some("ssh-ed25519 AAAAC3Nza test@box")
    );
    assert_eq!(public_key(&key.with_file_name("missing")), None);
    std::fs::remove_dir_all(key.parent().unwrap()).unwrap();
}
