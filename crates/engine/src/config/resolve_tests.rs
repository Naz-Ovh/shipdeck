use std::path::{Path, PathBuf};

use crate::config::load::parse_deploy;
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

use super::*;

fn deployment(yaml: &str) -> Deployment {
    let deploy = parse_deploy(yaml).unwrap();
    let endpoint = Endpoint::from_config(&deploy.server.ssh, HostConfig::default()).unwrap();
    Deployment {
        slug: "prod".into(),
        file: PathBuf::from("/home/me/shop/deploy/prod.yaml"),
        project: PathBuf::from("/home/me/shop"),
        config_dir: PathBuf::from("/home/me/shop/deploy"),
        yaml: yaml.to_string(),
        endpoint,
        deploy,
    }
}

const SAMPLE: &str = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    files: [dist]
    process: { start: node dist/main.js, systemd: {} }
  admin:
    source: apps/admin
    files: [apps/admin/dist]
    static: true
";

#[test]
fn paths_and_process_names_follow_the_design() {
    let deployment = deployment(SAMPLE);
    assert_eq!(deployment.dir(), "/opt/shop");
    assert_eq!(deployment.process_name("web"), "shop-web");
    assert_eq!(
        deployment.source("admin"),
        Path::new("/home/me/shop/apps/admin")
    );
    assert_eq!(deployment.release_dir("admin"), "apps/admin");
    assert_eq!(deployment.unit_file("web"), ".shipdeck/shop-web.service");
    assert_eq!(deployment.pm2_file("web"), ".shipdeck/shop-web.pm2.json");
    assert_eq!(deployment.app_names(), ["admin", "web"]);
    assert!(deployment.is_static("admin"));
    assert!(!deployment.is_static("web"));
    assert_eq!(
        deployment.manager("web").map(|manager| manager.kind()),
        Some(ManagerKind::Systemd)
    );
    assert_eq!(deployment.manager("admin"), None);
}

#[test]
fn the_manager_carries_its_own_block() {
    let yaml = SAMPLE.replace(
        "process: { start: node dist/main.js, systemd: {} }",
        "process:\n      start: node dist/main.js\n      systemd: { user: web }",
    );
    let systemd_deploy = deployment(&yaml);
    let Some(crate::config::Manager::Systemd(settings)) = systemd_deploy.manager("web") else {
        panic!("not systemd: {:?}", systemd_deploy.manager("web"));
    };
    assert_eq!(settings.user.as_deref(), Some("web"));
    assert_eq!(systemd_deploy.run_user("web"), "web");

    let pm2 = SAMPLE.replace(
        "process: { start: node dist/main.js, systemd: {} }",
        "process: { start: node dist/main.js, pm2: { options: { instances: 2 } } }",
    );
    let pm2_deploy = deployment(&pm2);
    let Some(crate::config::Manager::Pm2(settings)) = pm2_deploy.manager("web") else {
        panic!("not PM2: {:?}", pm2_deploy.manager("web"));
    };
    assert_eq!(
        settings.options.get("instances"),
        Some(&serde_json::json!(2))
    );
}

#[test]
fn a_process_name_can_be_overridden() {
    let yaml = SAMPLE.replace(
        "process: { start: node dist/main.js, systemd: {} }",
        "process:\n      name: api\n      start: node dist/main.js\n      systemd: {}",
    );
    let deployment = deployment(&yaml);
    assert_eq!(deployment.process_name("web"), "api");
}

#[test]
fn defaults_are_the_designs() {
    let deployment = deployment(SAMPLE);
    assert_eq!(
        deployment.keep(),
        Keep {
            open: 5,
            compressed: 15
        }
    );
    assert_eq!(deployment.rollback_mode(), RollbackMode::Ask);
    assert_eq!(deployment.ask_secs(), 30);
    assert!(!deployment.keep_failed());
    assert_eq!(deployment.health_timeout(), 20);
    assert_eq!(deployment.run_user("web"), "deploy");
}

#[test]
fn the_in_release_start_is_the_default_binary() {
    let yaml = SAMPLE.replace(
        "    process: { start: node dist/main.js, systemd: {} }",
        "    binary: target/release/shop\n    process: { systemd: {} }",
    );
    let deployment = deployment(&yaml);
    assert_eq!(deployment.binary_name("web").as_deref(), Some("shop"));
    assert_eq!(
        deployment.start("web"),
        Some((Program::InRelease("shop".into()), String::new()))
    );
}

#[test]
fn a_binary_with_to_sets_its_place_and_the_default_start() {
    let yaml = SAMPLE
        .replace(
            "    files: [dist]",
            "    files: [dist]\n    binary: { path: target/release/shop, to: bin/shop }",
        )
        .replace("start: node dist/main.js, ", "");
    let mapped = deployment(&yaml);
    assert_eq!(mapped.binary_name("web").as_deref(), Some("shop"));
    assert_eq!(mapped.binary_place("web").as_deref(), Some("bin/shop"));
    assert_eq!(
        mapped.start("web"),
        Some((Program::InRelease("bin/shop".into()), String::new()))
    );
    let plain = SAMPLE
        .replace(
            "    files: [dist]",
            "    files: [dist]\n    binary: target/release/shop",
        )
        .replace("start: node dist/main.js, ", "");
    let plain = deployment(&plain);
    assert_eq!(plain.binary_place("web").as_deref(), Some("shop"));
    assert_eq!(
        plain.start("web"),
        Some((Program::InRelease("shop".into()), String::new()))
    );
    // The map form without `to` behaves like the plain form.
    let bare = SAMPLE
        .replace(
            "    files: [dist]",
            "    files: [dist]\n    binary: { path: target/release/shop }",
        )
        .replace("start: node dist/main.js, ", "");
    let bare = deployment(&bare);
    assert_eq!(bare.binary_name("web").as_deref(), Some("shop"));
    assert_eq!(bare.binary_place("web").as_deref(), Some("shop"));
    assert_eq!(
        bare.start("web"),
        Some((Program::InRelease("shop".into()), String::new()))
    );
}
