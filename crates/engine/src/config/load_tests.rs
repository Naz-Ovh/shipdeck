use std::path::{Path, PathBuf};

use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "shipdeck-load-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const DEPLOY: &str = "\
name: shop
server: { ssh: { host: example.com, user: deploy }, dir: /opt/shop }
apps:
  web:
    process: { start: node dist/main.js, systemd: {} }
";

#[test]
fn the_project_file_names_the_deploy_files() {
    let dir = scratch("project");
    std::fs::write(
        dir.join("shipdeck.yaml"),
        "deploys:\n  production: deploy/production.yaml\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("deploy")).unwrap();
    std::fs::write(dir.join("deploy/production.yaml"), DEPLOY).unwrap();
    let project = Project::read(&dir).unwrap();
    assert_eq!(
        project.deploy_file("production"),
        Some(dir.join("deploy/production.yaml").as_path())
    );
    let deployment = load_deploy(
        &dir,
        project.deploy_file("production").unwrap(),
        Some("production"),
    )
    .unwrap();
    assert_eq!(deployment.slug, "production");
    assert_eq!(deployment.name(), "shop");
    assert_eq!(deployment.dir(), "/opt/shop");
    assert_eq!(deployment.process_name("web"), "shop-web");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_without_a_project_file_names_what_it_expected() {
    let dir = scratch("missing");
    let problems = Project::read(&dir).unwrap_err();
    let text = problems.problems[0].message.clone();
    assert!(text.contains("shipdeck.yaml"), "{text}");
    assert!(text.contains(".shipdeck"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn both_project_file_names_are_an_error() {
    let dir = scratch("both");
    std::fs::write(dir.join("shipdeck.yaml"), "deploys: {}\n").unwrap();
    std::fs::write(dir.join(".shipdeck"), "deploys: {}\n").unwrap();
    let problems = Project::read(&dir).unwrap_err();
    assert!(
        problems.problems[0].message.contains("both"),
        "{problems:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shipdeck_yml_is_not_read() {
    let dir = scratch("yml");
    std::fs::write(dir.join("shipdeck.yml"), "deploys: {}\n").unwrap();
    assert!(Project::read(&dir).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_deploy_file_lives_anywhere_and_needs_no_project_file() {
    let dir = scratch("file");
    let file = dir.join("production.yaml");
    std::fs::write(&file, DEPLOY).unwrap();
    let deployment = load_deploy(&dir, &file, None).unwrap();
    assert_eq!(deployment.slug, "shop");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_old_config_is_refused_by_its_unknown_keys() {
    let old = "\
name: Web
dir: /opt/web
ssh: { host: example.com, user: deploy }
service: { name: web }
";
    let problems = parse_deploy(old).unwrap_err();
    assert!(
        problems.iter().any(|p| p.message.contains("unknown key")),
        "{problems:?}"
    );
}

#[test]
fn a_deploy_can_be_read_back_from_its_settings() {
    let dir = scratch("settings");
    let file = dir.join("shop.yaml");
    std::fs::write(&file, DEPLOY).unwrap();
    let deployment = load_deploy(&dir, &file, Some("shop")).unwrap();
    let settings = deployment.settings().unwrap();
    let restored = Deployment::from_settings(
        &settings,
        Path::new("/tmp/project"),
        Path::new("/tmp/project"),
        file,
        "shop".into(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&restored.deploy).unwrap(),
        serde_json::to_value(&deployment.deploy).unwrap()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
