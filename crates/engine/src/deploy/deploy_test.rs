use std::path::PathBuf;

use tokio::sync::mpsc::unbounded_channel;

use super::*;
use crate::config::parse;
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

const NODE: &str = "
name: Web
ssh: { host: example.com, user: deploy }
build: { files: [dist] }
remote: { dir: /opt/web, service: web, start: node dist/main.js, requires: [npm] }
";

fn deployable(yaml: &str) -> Deployable {
    let config = parse(yaml).unwrap();
    Deployable {
        folder: PathBuf::from("/nonexistent/deploy/web"),
        slug: "web".into(),
        endpoint: Endpoint::from_config(&config.ssh, HostConfig::default()).unwrap(),
        config,
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
        sudo_missing: Vec::new(),
        sudoers_line: String::new(),
        tools: tools
            .iter()
            .map(|(name, path)| (name.to_string(), path.map(str::to_string)))
            .collect(),
    }
}

fn reporter() -> Reporter {
    Reporter::new(1, unbounded_channel().0)
}

#[test]
fn hook_steps_appear_only_with_hooks() {
    let plain = deployable(NODE);
    assert_eq!(
        deploy_step_names(&plain.config),
        [
            "Preflight",
            "Build",
            "Pack",
            "Upload",
            "Switch",
            "Restart",
            "Health",
            "Rotate"
        ]
    );
    let hooked = deployable(&format!("{NODE}hooks: {{ server: [npm ci] }}\n"));
    let steps = deploy_steps(&hooked.config);
    assert_eq!(steps[4], Step::Prepare);
    assert_eq!(steps.len(), 9);
}

#[test]
fn missing_tools_stop_preflight() {
    let web = deployable(NODE);
    let found = facts(&[("node", Some("/usr/bin/node")), ("npm", None)]);
    let err = check_tools(&web, &found, &reporter()).unwrap_err();
    assert!(err.to_string().contains("cannot find npm"), "{err}");
    let found = facts(&[
        ("node", Some("/usr/bin/node")),
        ("npm", Some("/usr/bin/npm")),
    ]);
    assert!(check_tools(&web, &found, &reporter()).is_ok());
    assert_eq!(start_path(&web, &found).as_deref(), Some("/usr/bin/node"));
}

#[test]
fn a_start_program_under_home_is_refused_with_the_default_unit() {
    let web = deployable(NODE);
    let found = facts(&[
        (
            "node",
            Some("/home/deploy/.nvm/versions/node/v22.0.0/bin/node"),
        ),
        ("npm", Some("/usr/bin/npm")),
    ]);
    let err = check_tools(&web, &found, &reporter()).unwrap_err();
    assert!(err.to_string().contains("ProtectHome"), "{err}");
    // npm is only used by hooks over SSH, so its place does not matter.
    let found = facts(&[
        ("node", Some("/usr/bin/node")),
        ("npm", Some("/home/deploy/.local/bin/npm")),
    ]);
    assert!(check_tools(&web, &found, &reporter()).is_ok());
}
