use std::path::PathBuf;

use super::*;
use crate::config::parse;
use crate::deploy::{deploy_steps, rollback_steps};
use crate::ssh::ssh_config::HostConfig;

const SAMPLE: &str = "\
name: Demo
ssh: { host: example.com, user: deploy, port: 2222 }
service: { name: demo, dir: /opt/demo, start: node dist/main.js }
pipeline:
  pack:
    files:
      include:
        project: [dist]
";

fn deployable() -> Deployable {
    let config = parse(SAMPLE).unwrap();
    Deployable {
        folder: PathBuf::from("/nonexistent/deploy/api"),
        slug: "api".into(),
        endpoint: crate::ssh::Endpoint::from_config(&config.ssh, HostConfig::default()).unwrap(),
        config,
        yaml: SAMPLE.to_string(),
    }
}

#[test]
fn a_project_carries_what_a_screen_shows() {
    let info = deployable_info(&deployable());
    assert_eq!(info.slug, "api");
    assert_eq!(info.name, "Demo");
    assert_eq!(info.host, "example.com");
    assert_eq!(info.user.as_deref(), Some("deploy"));
    assert_eq!(info.port, 2222);
    assert_eq!(info.dir, "/opt/demo");
}

#[test]
fn steps_are_named_as_a_screen_shows_them() {
    let deployable = deployable();
    let names = steps(&deploy_steps(&deployable.config));
    assert_eq!(names.first().map(String::as_str), Some("Preflight"));
    assert!(names.contains(&"Health".to_string()));
    let rollback = steps(&rollback_steps(&deployable.config));
    assert_eq!(rollback.first().map(String::as_str), Some("Connect"));
}

#[test]
fn a_failure_choice_beats_the_config() {
    assert_eq!(on_failure(wire::OnFailure::Rollback), Rollback::Auto);
    assert_eq!(on_failure(wire::OnFailure::Keep), Rollback::Ask);
}

#[test]
fn a_status_keeps_its_three_parts() {
    let state = Status {
        dir_exists: true,
        current: Some("v1".into()),
        active: "active".into(),
    };
    let answer = status(&state);
    assert!(answer.dir_exists);
    assert_eq!(answer.current.as_deref(), Some("v1"));
    assert_eq!(answer.state, "active");
}
