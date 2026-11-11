use super::*;

#[test]
fn a_deploy_has_all_twelve_steps() {
    let names: Vec<&str> = deploy_steps().iter().map(|step| step.name()).collect();
    assert_eq!(
        names,
        [
            "Preflight",
            "Build",
            "Upload",
            "Setup",
            "Backup",
            "Switch",
            "Restart",
            "Nginx",
            "Health",
            "Done",
            "Rollback",
            "Failed"
        ]
    );
    assert_eq!(Step::ALL.len(), 12);
}

#[test]
fn a_rollback_puts_the_target_first() {
    let names: Vec<&str> = rollback_steps().iter().map(|step| step.name()).collect();
    assert_eq!(
        names,
        [
            "Preflight",
            "Rollback",
            "Setup",
            "Switch",
            "Restart",
            "Nginx",
            "Health",
            "Done",
            "Failed"
        ]
    );
}

#[test]
fn only_activation_failures_need_a_rollback() {
    assert!(Step::Switch.needs_rollback());
    assert!(Step::Restart.needs_rollback());
    assert!(Step::Nginx.needs_rollback());
    assert!(Step::Health.needs_rollback());
    assert!(!Step::Build.needs_rollback());
    assert!(!Step::Setup.needs_rollback());
    assert!(!Step::Done.needs_rollback());
}

#[test]
fn commands_are_release_first_then_apps_in_name_order() {
    use crate::config::load::parse_deploy;
    use crate::ssh::Endpoint;
    use crate::ssh::ssh_config::HostConfig;
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
release:
  steps:
    done:
      server: echo release
apps:
  web:
    steps:
      done:
        server: echo web
    process: { start: node x, systemd: {} }
  api:
    steps:
      done:
        server: echo api
    process: { start: node y, systemd: {} }
";
    let deploy = parse_deploy(yaml).unwrap();
    let deployment = crate::config::Deployment {
        slug: "prod".into(),
        file: "deploy.yaml".into(),
        project: ".".into(),
        config_dir: ".".into(),
        yaml: yaml.into(),
        endpoint: Endpoint::from_config(&deploy.server.ssh, HostConfig::default()).unwrap(),
        deploy,
    };
    let apps: Vec<Option<&str>> = scopes(&deployment, Step::Done)
        .iter()
        .map(|scope| scope.app)
        .collect();
    assert_eq!(apps, [None, Some("api"), Some("web")]);
}
