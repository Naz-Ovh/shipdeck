use std::path::{Path, PathBuf};

use crate::config::Deployment;
use crate::config::load::parse_deploy;
use crate::ssh::Endpoint;
use crate::ssh::ssh_config::HostConfig;

use super::{Options, run};

fn write(root: &Path, path: &str, text: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

fn deployment(root: &Path) -> Deployment {
    let yaml = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
release:
  files: [package.json]
apps:
  api:
    source: api
    files: [api/dist]
    binary: { path: api/target/api, to: bin/api }
    process: { systemd: {} }
    steps:
      health:
        url: http://127.0.0.1:3000/
";
    let deploy = parse_deploy(yaml).unwrap();
    Deployment {
        slug: "prod".into(),
        file: root.join("deploy.yaml"),
        project: root.to_path_buf(),
        config_dir: root.to_path_buf(),
        yaml: yaml.into(),
        endpoint: Endpoint::from_config(&deploy.server.ssh, HostConfig::default()).unwrap(),
        ssh_from: Vec::new(),
        unavailable: None,
        deploy,
    }
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("shipdeck-tree-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[tokio::test]
async fn a_binary_with_to_is_in_the_tree_at_its_place() {
    let root = scratch("binary");
    write(&root, "package.json", "{\"version\":\"1.0.0\"}");
    write(&root, "api/dist/server.js", "x");
    write(&root, "api/target/api", "bin");
    let deployment = deployment(&root);
    let outcome = run(
        &deployment,
        &Options {
            json: true,
            ..Options::default()
        },
    )
    .await
    .unwrap();
    assert!(
        outcome
            .text
            .lines()
            .any(|line| line.contains("\"path\":\"apps/api/bin/api\"")
                && line.contains("\"mode\":\"0755\"")),
        "{}",
        outcome.text
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn the_tree_folds_and_counts() {
    let root = scratch("fold");
    write(&root, "package.json", "{\"version\":\"1.0.0\"}");
    write(&root, "api/dist/a.js", "x");
    write(&root, "api/dist/deep/b.js", "x");
    write(&root, "api/target/api", "bin");
    let deployment = deployment(&root);
    let outcome = run(
        &deployment,
        &Options {
            depth: Some(3),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    assert!(outcome.text.contains("dist/ (2 files)"), "{}", outcome.text);
    assert!(outcome.text.contains("release.json"), "{}", outcome.text);
    assert!(outcome.text.contains("bin/ (1 files)"), "{}", outcome.text);
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn one_app_can_be_shown_alone() {
    let root = scratch("app");
    write(&root, "package.json", "{\"version\":\"1.0.0\"}");
    write(&root, "api/dist/a.js", "x");
    write(&root, "api/target/api", "bin");
    write(&root, "web/dist/index.html", "x");
    let deployment = deployment(&root);
    let outcome = run(
        &deployment,
        &Options {
            app: Some("api".into()),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    assert!(outcome.text.contains("dist/"), "{}", outcome.text);
    assert!(!outcome.text.contains("web/"), "{}", outcome.text);
    std::fs::remove_dir_all(&root).unwrap();
}
