use tokio::sync::mpsc::unbounded_channel;

use super::*;
use crate::config::Command;
use crate::config::load::parse_deploy;
use crate::deploy::commands::{Exec, runner_line};
use crate::deploy::report::Reporter;
use crate::remote::Place;
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

const SAMPLE: &str = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  api:
    process:
      start: node dist/main.js
      env: { NODE_ENV: production }
      env_file: .env
      systemd: {}
";

fn deployment() -> crate::config::Deployment {
    let deploy = parse_deploy(SAMPLE).unwrap();
    let endpoint = Endpoint::from_config(&deploy.server.ssh, HostConfig::default()).unwrap();
    crate::config::Deployment {
        slug: "prod".into(),
        file: "deploy.yaml".into(),
        project: ".".into(),
        config_dir: ".".into(),
        yaml: SAMPLE.into(),
        endpoint,
        deploy,
    }
}

fn reporter() -> Reporter {
    let (tx, _rx) = unbounded_channel();
    Reporter::new(1, tx)
}

#[test]
fn commands_see_the_step_placeholders_and_the_error() {
    let deployment = deployment();
    let report = reporter();
    let exec = Exec {
        deployment: &deployment,
        report: &report,
        session: None,
        release: Some("v1.4.0-abc1234-20260101.120000".into()),
        version: Some("1.4.0".into()),
        previous: Some("v1.3.9-def5678-20251231.120000".into()),
        failed_step: Some(Step::Health),
        action: super::super::steps::Action::Deploy,
        row: 0,
    };
    let env = exec.env(Step::Failed, Some("api"), Some("boom")).unwrap();
    let get = |key: &str| {
        env.vars
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(get("SHIPDECK_ACTION"), Some("deploy"));
    assert_eq!(get("SHIPDECK_STEP"), Some("health"));
    assert_eq!(
        get("SHIPDECK_PREVIOUS"),
        Some("v1.3.9-def5678-20251231.120000")
    );
    assert_eq!(get("SHIPDECK_ERROR"), Some("boom"));
    assert_eq!(
        get("SHIPDECK_APP_CURRENT"),
        Some("/opt/shop/current/apps/api")
    );
    assert_eq!(get("SHIPDECK_PROCESS"), Some("shop-api"));
    assert_eq!(env.env_file.as_deref(), Some("apps/api/.env"));
    assert_eq!(env.service["NODE_ENV"], "production");
}

#[test]
fn places_follow_the_step_table() {
    let deployment = deployment();
    let report = reporter();
    let exec = Exec {
        deployment: &deployment,
        report: &report,
        session: None,
        release: Some("v1".into()),
        version: None,
        previous: None,
        failed_step: None,
        action: super::super::steps::Action::Deploy,
        row: 0,
    };
    assert_eq!(exec.place(Step::Preflight, None).unwrap(), Place::Root);
    assert_eq!(exec.place(Step::Build, Some("api")).unwrap(), Place::Root);
    assert_eq!(
        exec.place(Step::Setup, Some("api")).unwrap(),
        Place::AppRelease("v1".into(), "api".into())
    );
    assert_eq!(
        exec.place(Step::Setup, None).unwrap(),
        Place::Release("v1".into())
    );
    assert_eq!(
        exec.place(Step::Health, Some("api")).unwrap(),
        Place::AppCurrent("api".into())
    );
    assert_eq!(exec.place(Step::Done, None).unwrap(), Place::Current);
    assert_eq!(exec.place(Step::Failed, None).unwrap(), Place::Root);
}

#[test]
fn a_runner_command_quotes_its_code() {
    assert_eq!(
        runner_line(crate::config::Runner::Node, "if (x) { process.exit(1) }"),
        "node -e 'if (x) { process.exit(1) }'"
    );
}

#[test]
fn a_server_command_gets_its_placeholders() {
    let deployment = deployment();
    let report = reporter();
    let exec = Exec {
        deployment: &deployment,
        report: &report,
        session: None,
        release: Some("v1.4.0-abc1234-20260101.120000".into()),
        version: Some("1.4.0".into()),
        previous: None,
        failed_step: None,
        action: super::super::steps::Action::Deploy,
        row: 0,
    };
    let command = Command::Sh("test -w {{ROOT}} && echo {{RELEASE_ID}}".into());
    assert_eq!(
        exec.line(&command, None).unwrap(),
        "test -w /opt/shop && echo v1.4.0-abc1234-20260101.120000"
    );
}

#[test]
fn a_local_command_gets_its_app_placeholders() {
    let deployment = deployment();
    let report = reporter();
    let exec = Exec {
        deployment: &deployment,
        report: &report,
        session: None,
        release: Some("v1.4.0-abc1234-20260101.120000".into()),
        version: Some("1.4.0".into()),
        previous: None,
        failed_step: None,
        action: super::super::steps::Action::Deploy,
        row: 0,
    };
    let command = Command::Runner {
        runner: crate::config::Runner::Node,
        code: "console.log('{{PROCESS}}')".into(),
    };
    let line = exec.line(&command, Some("api")).unwrap();
    assert!(line.starts_with("node -e "), "{line}");
    assert!(line.contains("shop-api"), "{line}");
    // The runner gets the code, not the whole shell line.
    assert_eq!(
        exec.code(&command, Some("api")).unwrap(),
        "console.log('shop-api')"
    );
    assert_eq!(
        exec.code(&Command::Sh("cd {{ROOT}}".into()), None).unwrap(),
        "cd /opt/shop"
    );
}

#[test]
fn an_env_value_gets_its_placeholders() {
    let text = SAMPLE.replace(
        "env: { NODE_ENV: production }",
        "env: { NODE_ENV: production, ROOT: '{{ROOT}}', APP: '{{APP_CURRENT}}' }",
    );
    let deploy = parse_deploy(&text).unwrap();
    let endpoint = Endpoint::from_config(&deploy.server.ssh, HostConfig::default()).unwrap();
    let deployment = crate::config::Deployment {
        slug: "prod".into(),
        file: "deploy.yaml".into(),
        project: ".".into(),
        config_dir: ".".into(),
        yaml: text.clone(),
        endpoint,
        deploy,
    };
    let report = reporter();
    let exec = Exec {
        deployment: &deployment,
        report: &report,
        session: None,
        release: Some("v1.4.0-abc1234-20260101.120000".into()),
        version: Some("1.4.0".into()),
        previous: None,
        failed_step: None,
        action: super::super::steps::Action::Deploy,
        row: 0,
    };
    let env = exec.env(Step::Setup, Some("api"), None).unwrap();
    assert_eq!(env.service["ROOT"], "/opt/shop");
    assert_eq!(env.service["APP"], "/opt/shop/current/apps/api");
    assert_eq!(env.service["NODE_ENV"], "production");
}

#[test]
fn a_short_error_is_the_first_line() {
    let error = anyhow::anyhow!("the first line\nand a second one");
    assert_eq!(super::super::short(&error), "the first line");
}

#[test]
fn the_env_file_is_relative_to_the_app_folder() {
    let deployment = deployment();
    assert_eq!(deployment.env_file("api").as_deref(), Some(".env"));
    assert_eq!(deployment.app_env("api").len(), 1);
}
