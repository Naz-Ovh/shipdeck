use std::path::PathBuf;

use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::report::{OpEvent, OpUpdate};
use super::*;
use crate::config::parse;
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

const NODE: &str = "
name: Web
ssh: { host: example.com, user: deploy }
service: { name: web, dir: /opt/web, start: node dist/main.js }
pipeline:
  preflight: { requires: [npm] }
  pack:
    files:
      include:
        project: [dist]
";

fn deployable(yaml: &str) -> Deployable {
    let config = parse(yaml).unwrap();
    Deployable {
        folder: PathBuf::from("/nonexistent/deploy/web"),
        slug: "web".into(),
        endpoint: Endpoint::from_config(&config.ssh, HostConfig::default()).unwrap(),
        config,
        yaml: yaml.to_string(),
    }
}

fn facts(tools: &[(&str, Option<&str>)]) -> Facts {
    Facts {
        arch: "x86_64".into(),
        glibc: None,
        dir_exists: true,
        dir_writable: true,
        free_bytes: None,
        current: None,
        active: String::new(),
        systemd: String::new(),
        pm2_version: None,
        sudo_missing: Vec::new(),
        tools: tools
            .iter()
            .map(|(name, path)| (name.to_string(), path.map(str::to_string)))
            .collect(),
    }
}

fn reporter() -> (Reporter, UnboundedReceiver<OpUpdate>) {
    let (tx, rx) = unbounded_channel();
    (Reporter::new(1, tx), rx)
}

#[test]
fn every_deploy_has_the_eleven_steps_and_failed_only_with_commands() {
    let plain = deployable(NODE);
    assert_eq!(
        deploy_step_names(&plain.config),
        [
            "Preflight",
            "Build",
            "Pack",
            "Upload",
            "Unpack",
            "Setup",
            "Backup",
            "Switch",
            "Restart",
            "Health",
            "Done"
        ]
    );
    assert_eq!(
        rollback_step_names(&plain.config),
        [
            "Connect", "Unpack", "Setup", "Switch", "Restart", "Health", "Done"
        ]
    );
    let alerting = deployable(&format!(
        "{NODE}  failed: {{ run: {{ local: ./alert }} }}\n"
    ));
    assert_eq!(deploy_steps(&alerting.config).last(), Some(&Step::Failed));
    assert_eq!(rollback_steps(&alerting.config).len(), 8);
}

#[test]
fn commands_see_what_is_going_on() {
    let web = deployable(NODE);
    let (report, _rx) = reporter();
    let mut run = Run::new(&web, &report, deploy_steps(&web.config), Action::Rollback);
    run.release = Some("v1.4.0-abc1234-20261014.120000".into());
    run.version = Some("1.4.0".into());
    run.previous = Some("v1.3.9-def5678-20261013.120000".into());
    let vars = run.vars(Step::Health, Some("http 502"));
    let get = |key: &str| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("SHIPDECK_ACTION"), Some("rollback"));
    assert_eq!(get("SHIPDECK_STEP"), Some("health"));
    assert_eq!(get("SHIPDECK_VERSION"), Some("1.4.0"));
    assert_eq!(
        get("SHIPDECK_PREVIOUS"),
        Some("v1.3.9-def5678-20261013.120000")
    );
    assert_eq!(get("SHIPDECK_SERVICE"), Some("web"));
    assert_eq!(get("SHIPDECK_SHARED_DIR"), Some("/opt/web/shared"));
    assert_eq!(get("SHIPDECK_ERROR"), Some("http 502"));
}

#[test]
fn skipping_the_rest_leaves_failed_to_run() {
    let web = deployable(&format!(
        "{NODE}  failed: {{ run: {{ local: ./alert }} }}\n"
    ));
    let (report, mut rx) = reporter();
    let run = Run::new(&web, &report, deploy_steps(&web.config), Action::Deploy);
    run.skip_rest(Step::Health, "rolled back");
    let mut skipped = Vec::new();
    while let Ok(update) = rx.try_recv() {
        if let OpEvent::Step { step, .. } = update.event {
            skipped.push(step);
        }
    }
    // Done (index 10) is skipped; Failed (index 11) is not.
    assert_eq!(skipped, [10]);
}

#[tokio::test]
async fn the_rollback_question_takes_the_answer_given() {
    let web = deployable(NODE);
    let (report, mut rx) = reporter();
    let mut run = Run::new(&web, &report, deploy_steps(&web.config), Action::Deploy);
    run.release = Some("v1.4.0-abc1234-20261014.120000".into());
    let answer = |keep: bool| async move { keep };
    let err = anyhow!("http 502 for 20s");

    for give in [false, true] {
        let asked = run.should_roll_back(Step::Health, &err, "v1.3.9");
        let answering = async {
            loop {
                let update = rx.recv().await.unwrap();
                if let OpEvent::AskRollback(question) = update.event {
                    assert_eq!(question.previous, "v1.3.9");
                    assert_eq!(question.step, "Health");
                    assert_eq!(question.seconds, ROLLBACK_ANSWER_SECS);
                    question.reply.send(answer(give).await).unwrap();
                    break;
                }
            }
        };
        let (rolls_back, ()) = tokio::join!(asked, answering);
        assert_eq!(rolls_back, give);
    }
}

#[tokio::test]
async fn with_nobody_to_ask_or_auto_it_rolls_back() {
    let web = deployable(NODE);
    // The screen is gone: the question cannot be shown.
    let (report, rx) = reporter();
    drop(rx);
    let run = Run::new(&web, &report, deploy_steps(&web.config), Action::Deploy);
    assert!(
        run.should_roll_back(Step::Restart, &anyhow!("x"), "v1")
            .await
    );

    let auto = deployable(&format!("{NODE}  failed: {{ rollback: auto }}\n"));
    let (report, mut rx) = reporter();
    let run = Run::new(&auto, &report, deploy_steps(&auto.config), Action::Deploy);
    assert!(
        run.should_roll_back(Step::Health, &anyhow!("x"), "v1")
            .await
    );
    while let Ok(update) = rx.try_recv() {
        assert!(
            !matches!(update.event, OpEvent::AskRollback(_)),
            "auto never asks"
        );
    }
}

#[test]
fn missing_tools_stop_preflight() {
    let web = deployable(NODE);
    let (report, _rx) = reporter();
    let found = facts(&[("node", Some("/usr/bin/node")), ("npm", None)]);
    let err = check_tools(&web, &found, &report).unwrap_err();
    assert!(err.to_string().contains("cannot find npm"), "{err}");
    let found = facts(&[
        ("node", Some("/usr/bin/node")),
        ("npm", Some("/usr/bin/npm")),
    ]);
    assert!(check_tools(&web, &found, &report).is_ok());
    assert_eq!(start_path(&web, &found).as_deref(), Some("/usr/bin/node"));
}

#[test]
fn a_server_without_zstd_is_told_to_install_it() {
    let web = deployable(NODE);
    let (report, _rx) = reporter();
    let found = facts(&[
        ("node", Some("/usr/bin/node")),
        ("npm", None),
        ("tar", Some("/usr/bin/tar")),
        ("zstd", None),
    ]);
    let err = check_tools(&web, &found, &report).unwrap_err().to_string();
    // shipdeck's own tool comes first, with the package to install.
    assert!(err.contains("has no zstd"), "{err}");
    assert!(
        err.contains("apt install zstd, or dnf install zstd"),
        "{err}"
    );
}

#[test]
fn a_start_program_under_home_is_refused_with_the_default_unit() {
    let web = deployable(NODE);
    let (report, _rx) = reporter();
    let found = facts(&[
        (
            "node",
            Some("/home/deploy/.nvm/versions/node/v22.0.0/bin/node"),
        ),
        ("npm", Some("/usr/bin/npm")),
    ]);
    let err = check_tools(&web, &found, &report).unwrap_err();
    assert!(err.to_string().contains("ProtectHome"), "{err}");
    // npm is only used by commands over SSH, so its place does not matter.
    let found = facts(&[
        ("node", Some("/usr/bin/node")),
        ("npm", Some("/home/deploy/.local/bin/npm")),
    ]);
    assert!(check_tools(&web, &found, &report).is_ok());
}

#[test]
fn pm2_must_be_known_and_alone() {
    let web = deployable(&NODE.replace(
        "start: node dist/main.js",
        "start: node dist/main.js, manager: pm2",
    ));
    let (report, _rx) = reporter();
    let mut found = facts(&[]);
    found.pm2_version = Some("6.0.8".into());
    check_pm2(&web, &found, &report).unwrap();

    found.pm2_version = Some(format!("{}.0.0", crate::remote::PM2_NEWEST_MAJOR + 1));
    let err = check_pm2(&web, &found, &report).unwrap_err().to_string();
    assert!(err.contains("newer than this shipdeck supports"), "{err}");

    found.pm2_version = Some("6.0.8".into());
    found.systemd = "active".into();
    let err = check_pm2(&web, &found, &report).unwrap_err().to_string();
    assert!(err.contains("sudo systemctl disable --now web"), "{err}");
}
