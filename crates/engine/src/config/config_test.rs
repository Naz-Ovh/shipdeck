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
    assert_eq!(config.ssh.port, None);
    assert_eq!(config.ssh.user.as_deref(), Some("deploy"));
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

fn with_bundle(entries: &[&str]) -> DeployConfig {
    let mut config = sample();
    config.bundle = entries.iter().map(|e| e.to_string()).collect();
    config
}

#[test]
fn rejects_bundle_entries_naming_the_whole_folder() {
    for entry in ["", ".", "./", "./."] {
        let err = with_bundle(&[entry]).validate().unwrap_err();
        assert!(
            err.to_string().contains("whole deployable folder"),
            "{entry:?}"
        );
    }
}

#[test]
fn rejects_duplicate_and_overlapping_bundle_entries() {
    let err = with_bundle(&[".env", "./.env"]).validate().unwrap_err();
    assert!(err.to_string().contains("twice"));
    let err = with_bundle(&["files/", "files/a.txt"])
        .validate()
        .unwrap_err();
    assert!(err.to_string().contains("overlap"));
    let err = with_bundle(&["files/a.txt", "files"])
        .validate()
        .unwrap_err();
    assert!(err.to_string().contains("overlap"));
    // A shared prefix is not an overlap.
    assert!(
        with_bundle(&["files", "files-extra", ".env"])
            .validate()
            .is_ok()
    );
}

#[test]
fn rejects_bundle_entries_that_replace_release_files() {
    for entry in ["release.json", "demo", "./demo.service"] {
        let err = with_bundle(&[entry]).validate().unwrap_err();
        assert!(err.to_string().contains("would replace"), "{entry}");
    }
    // The same names deeper down are fine.
    assert!(with_bundle(&["files/release.json"]).validate().is_ok());
}

#[test]
fn bundle_paths_are_stored_without_dots_or_slashes() {
    assert_eq!(bundle_path("./files/"), "files");
    assert_eq!(bundle_path("files/./sub/a.txt"), "files/sub/a.txt");
    assert_eq!(bundle_path("."), "");
}

#[test]
fn check_bundle_lists_every_missing_entry() {
    let dir = std::env::temp_dir().join(format!("shipdeck-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let deployable = Deployable {
        folder: dir.clone(),
        slug: "demo".into(),
        config: with_bundle(&[".env", "files/", "certs/key.pem"]),
        endpoint: crate::ssh::Endpoint::from_config(
            &sample().ssh,
            crate::ssh::ssh_config::HostConfig::default(),
        )
        .unwrap(),
    };
    let err = deployable.check_bundle().unwrap_err().to_string();
    assert!(err.starts_with("2 of 3 bundle entries"), "{err}");
    assert!(
        err.contains(".env: missing") && err.contains("certs/key.pem: missing"),
        "{err}"
    );

    std::fs::write(dir.join(".env"), "A=1").unwrap();
    std::fs::create_dir_all(dir.join("certs")).unwrap();
    std::fs::write(dir.join("certs/key.pem"), "k").unwrap();
    assert!(deployable.check_bundle().is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn user_may_come_from_ssh_config() {
    let config: DeployConfig = serde_json::from_str(
        r#"{
            "name": "Demo",
            "ssh": { "host": "prod" },
            "build": { "command": ["cargo", "build"], "binary": "target/release/demo" },
            "remote": { "dir": "/opt/demo", "service": "demo" }
        }"#,
    )
    .unwrap();
    assert!(config.validate().is_ok());
    assert_eq!(config.ssh.user, None);
}
