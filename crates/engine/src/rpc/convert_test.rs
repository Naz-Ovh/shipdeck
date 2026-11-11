use std::path::PathBuf;

use super::*;
use crate::config::load::parse_deploy;
use crate::deploy::{deploy_plan, rollback_plan};
use crate::ssh::ssh_config::HostConfig;

const SAMPLE: &str = "\
name: Demo
server: { ssh: { host: example.com, user: deploy, port: 2222 }, dir: /opt/demo }
apps:
  api:
    source: services/api
    process: { start: node dist/main.js, systemd: {} }
  web:
    static: true
    files: [web/dist]
";

fn deployment() -> Deployment {
    let deploy = parse_deploy(SAMPLE).unwrap();
    Deployment {
        slug: "api".into(),
        file: PathBuf::from("/work/deploy/production.yaml"),
        project: PathBuf::from("/work"),
        config_dir: PathBuf::from("/work/deploy"),
        yaml: SAMPLE.into(),
        endpoint: crate::ssh::Endpoint::from_config(&deploy.server.ssh, HostConfig::default())
            .unwrap(),
        ssh_from: Vec::new(),
        unavailable: None,
        deploy,
    }
}

#[test]
fn a_deploy_carries_what_a_screen_shows() {
    let info = deployable_info(&deployment());
    assert_eq!(info.slug, "api");
    assert_eq!(info.name, "Demo");
    assert_eq!(info.host, "example.com");
    assert_eq!(info.user.as_deref(), Some("deploy"));
    assert_eq!(info.port, 2222);
    assert_eq!(info.dir, "/opt/demo");
    assert_eq!(info.apps, ["api", "web"]);
}

#[test]
fn steps_are_named_as_a_screen_shows_them() {
    let names: Vec<&str> = deploy_plan().iter().map(|step| step.name()).collect();
    assert_eq!(names.first().copied(), Some("Preflight"));
    assert!(names.contains(&"Health"));
    let rollback: Vec<&str> = rollback_plan().iter().map(|step| step.name()).collect();
    assert_eq!(rollback.first().copied(), Some("Preflight"));
    assert!(rollback.contains(&"Rollback"));
}

#[test]
fn a_failure_choice_beats_the_config() {
    assert_eq!(on_failure(wire::OnFailure::Rollback), RollbackMode::Auto);
    assert_eq!(on_failure(wire::OnFailure::Keep), RollbackMode::Off);
}

#[test]
fn no_flag_leaves_the_deploy_files_mode_alone() {
    let yaml = SAMPLE.replace(
        "name: Demo",
        "name: Demo\nrelease: { steps: { rollback: { mode: off } } }",
    );
    let mut deployment = deployment();
    deployment.deploy = parse_deploy(&yaml).unwrap();
    apply_on_failure(&mut deployment, None);
    assert_eq!(deployment.rollback_mode(), RollbackMode::Off);
}

#[test]
fn a_flag_still_wins_over_the_deploy_file() {
    let yaml = SAMPLE.replace(
        "name: Demo",
        "name: Demo\nrelease: { steps: { rollback: { mode: off } } }",
    );
    let mut deployment = deployment();
    deployment.deploy = parse_deploy(&yaml).unwrap();
    apply_on_failure(&mut deployment, Some(wire::OnFailure::Rollback));
    assert_eq!(deployment.rollback_mode(), RollbackMode::Auto);
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
