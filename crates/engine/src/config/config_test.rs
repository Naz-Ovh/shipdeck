use super::*;

fn sample() -> DeployConfig {
    serde_json::from_str(
        r#"{
            "name": "Demo",
            "ssh": { "host": "example.com", "user": "deploy" },
            "build": { "command": ["cargo", "build"], "binary": "target/release/demo" },
            "remote": { "dir": "/opt/demo", "service": "demo" }
        }"#,
    )
    .unwrap()
}

#[test]
fn fills_in_defaults() {
    let config = sample();
    assert_eq!(config.ssh.port, 22);
    assert_eq!(config.build.cwd, ".");
    assert_eq!(
        config.releases,
        Retention {
            open: 5,
            compressed: 15
        }
    );
    assert_eq!(config.health.timeout_secs, 20);
    assert!(config.validate().is_ok());
}

#[test]
fn rejects_unknown_fields() {
    let err = serde_json::from_str::<DeployConfig>(
        r#"{
            "name": "Demo", "colour": "red",
            "ssh": { "host": "example.com", "user": "deploy" },
            "build": { "command": ["cargo"], "binary": "x" },
            "remote": { "dir": "/opt/demo", "service": "demo" }
        }"#,
    );
    assert!(err.is_err());
}

#[test]
fn rejects_unsafe_remote_dir() {
    for dir in ["opt/demo", "/", "/opt/../etc", "/opt/my app", "/opt/$(id)"] {
        let mut config = sample();
        config.remote.dir = dir.into();
        assert!(config.validate().is_err(), "{dir} should be rejected");
    }
}

#[test]
fn rejects_service_with_suffix_or_shell_characters() {
    for service in ["demo.service", "demo;rm", "", "a b"] {
        let mut config = sample();
        config.remote.service = service.into();
        assert!(config.validate().is_err(), "{service:?} should be rejected");
    }
}

#[test]
fn rejects_bundle_paths_outside_the_folder() {
    for entry in ["../secret", "/etc/passwd", "files/../../x"] {
        let mut config = sample();
        config.bundle = vec![entry.into()];
        assert!(config.validate().is_err(), "{entry} should be rejected");
    }
}

#[test]
fn rejects_zero_open_releases() {
    let mut config = sample();
    config.releases.open = 0;
    assert!(config.validate().is_err());
}

#[test]
fn ssh_label_hides_the_default_port() {
    let mut target = sample().ssh;
    assert_eq!(target.label(), "deploy@example.com");
    target.port = 2222;
    assert_eq!(target.label(), "deploy@example.com:2222");
}
